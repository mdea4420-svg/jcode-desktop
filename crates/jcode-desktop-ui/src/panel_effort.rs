//! Reasoning effort switching for a chat panel: `/effort`, the effort pill
//! menu, and the fast increase/decrease keys all funnel through here.
//!
//! A switch is shown immediately (the pill moves on the keypress) and then
//! settled by the runtime: a refusal restores the previous level, and the
//! identity broadcast that follows a success carries the level the provider
//! actually applied.
use super::*;
use crate::custom_providers::MagpieEffort;
use crate::effort;
use crate::locale::tr;
use std::{cell::Cell, rc::Rc};

pub(super) type PillBounds = Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>;

pub(super) fn pill_bounds_marker(bounds: PillBounds) -> gpui::AnyElement {
    gpui::canvas(
        move |rect, window, _| {
            let visible = rect.intersect(&window.content_mask().bounds);
            bounds.set((visible.size.width > px(0.) && visible.size.height > px(0.)).then_some(visible));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}


/// An effort request the runtime has not answered yet.
#[derive(Clone, Debug)]
pub(super) struct PendingEffort {
    /// What the pill showed before the first unanswered request.
    previous: Option<String>,
    /// Requests sent and not yet settled, oldest first.
    requested: VecDeque<String>,
}

pub(super) struct EffortPicker {
    focus: gpui::FocusHandle,
    selected: Option<&'static str>,
    pill_bounds: PillBounds,
}

impl Panel {
    pub(super) fn toggle_effort_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_side_document() || self.is_pending_session() {
            return;
        }
        if self.effort_picker.is_some() {
            self.close_effort_picker(window, cx);
            return;
        }
        if self.model_picker_open {
            self.close_model_picker(cx);
        }
        self.model_browser = None;
        self.swarm.focus = None;
        self.recovery_picker_open = false;
        self.close_login_picker(cx);
        if matches!(self.magpie_effort, Some(MagpieEffort::Unknown)) {
            self.magpie_effort_model = None;
            self.refresh_magpie_effort(cx);
        }
        let levels = self.effort_ladder();
        let selected = levels
            .iter()
            .copied()
            .find(|level| self.reasoning_effort.as_deref() == Some(*level))
            .or_else(|| levels.first().copied());
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.effort_picker = Some(EffortPicker {
            focus,
            selected,
            pill_bounds: Default::default(),
        });
        cx.notify();
    }

    fn close_effort_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.effort_picker = None;
        self.focus_input(window, cx);
        cx.notify();
    }

    fn choose_effort_level(&mut self, level: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.reasoning_effort.as_deref() != Some(level) {
            self.request_effort(level, cx);
        }
        self.close_effort_picker(window, cx);
    }

    pub(super) fn effort_pill_bounds_marker(&self) -> Option<gpui::AnyElement> {
        self.effort_picker
            .as_ref()
            .map(|picker| pill_bounds_marker(picker.pill_bounds.clone()))
    }

    pub(super) fn render_effort_picker(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let picker = self.effort_picker.as_ref()?;
        let pill_bounds = picker.pill_bounds.clone();
        let levels = self.effort_ladder();
        let theme = Theme::global();
        let rows = levels.iter().copied().map(|level| {
            let current = self.reasoning_effort.as_deref() == Some(level);
            div()
                .id(gpui::SharedString::from(format!("effort-level-{level}")))
                .debug_selector(move || format!("effort-level-{level}").into())
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3()
                .py_1()
                .rounded_full()
                .cursor_pointer()
                .bg(theme.TOOL_BG)
                .when(picker.selected == Some(level), |row| row.bg(theme.ACCENT_DIM))
                .hover(|row| row.bg(theme.ACCENT_DIM))
                .child(effort_level_label(level))
                .when(current, |row| row.child(tr("Current", "当前")))
                .when(self.custom_provider_default_effort() == Some(level), |row| {
                    row.child(tr("Gateway default", "网关默认"))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_effort_level(level, window, cx);
                    cx.stop_propagation();
                }))
        });
        Some(
            div()
                .id("effort-picker")
                .debug_selector(|| "effort-picker".into())
                .track_focus(&picker.focus)
                .max_h(super::composer::composer_picker_max_height(window))
                .overflow_y_scroll()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex()
                .flex_col()
                .gap_1()
                .mb_2()
                .p_2()
                .rounded_xl()
                .bg(theme.HEADER_BG)
                .text_color(theme.TEXT)
                .text_size(px(12.))
                .child(tr("Reasoning effort", "推理强度"))
                .children(rows)
                .when(levels.is_empty(), |menu| menu.child(self.effort_unavailable_message()))
                .on_mouse_down_out(cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    // Let the pill's MouseUp toggle close an already-open picker.
                    if this.effort_pill_label().is_some()
                        && pill_bounds.get().is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        return;
                    }
                    this.effort_picker = None;
                    cx.notify();
                }))
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    let selected = this.effort_picker.as_ref().and_then(|picker| picker.selected);
                    match event.keystroke.key.as_str() {
                        "escape" => this.close_effort_picker(window, cx),
                        "enter" => {
                            if let Some(level) = selected {
                                this.choose_effort_level(level, window, cx);
                            }
                        }
                        key @ ("up" | "down") => {
                            let levels = this.effort_ladder();
                            let next = effort::step(&levels, selected, if key == "up" { -1 } else { 1 });
                            if let (Some(picker), Some(level)) = (&mut this.effort_picker, next) {
                                picker.selected = Some(level);
                                cx.notify();
                            }
                        }
                        _ => return,
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                }))
                .into_any_element(),
        )
    }

    /// Levels the serving model accepts.
    pub(super) fn effort_ladder(&self) -> Vec<&'static str> {
        if self.uses_custom_provider_effort() {
            return match &self.magpie_effort {
                Some(MagpieEffort::Known { levels, fixed: false, .. }) => levels.clone(),
                _ => Vec::new(),
            };
        }
        effort::ladder(self.provider.as_deref(), self.model.as_deref())
    }

    fn uses_custom_provider_effort(&self) -> bool {
        self.magpie_effort_model.is_some() || self.provider.as_deref() == Some("magpie")
    }

    fn custom_provider_effort_target<'a>(&'a self, cx: &'a App) -> Option<(&'a str, &'a str)> {
        let model = self.model.as_deref()?;
        let provider = self.provider.as_deref();
        let input = self.input.read(cx);
        let is_configured = |profile: &str| {
            profile == "magpie" || input.is_configured_model_profile(profile)
        };
        let route = input.route_for_model(model, provider).filter(|(_, route)| {
            provider.is_none_or(|provider| route.provider.eq_ignore_ascii_case(provider))
        });
        let profile = route
            .and_then(|(_, route)| route.api_method.strip_prefix("openai-compatible:"))
            .filter(|profile| is_configured(profile))
            .or_else(|| {
                let provider = provider?;
                let profile = provider.strip_prefix("openai-compatible:").unwrap_or(provider);
                is_configured(profile).then_some(profile)
            })?;
        let id = crate::input::custom_profile_model_id(model, profile);
        (!id.is_empty()).then_some((profile, id))
    }

    // Keep the existing Panel entry point while probing all configured endpoints.
    pub(super) fn refresh_magpie_effort(&mut self, cx: &mut Context<Self>) {
        let Some((profile, model)) = self.custom_provider_effort_target(cx) else {
            self.magpie_effort_model = None;
            self.magpie_effort = None;
            return;
        };
        let route_key = format!("{profile}:{model}");
        if self.magpie_effort_model.as_deref() == Some(route_key.as_str()) {
            return;
        }
        let (profile, model) = (profile.to_owned(), model.to_owned());
        self.magpie_effort_model = Some(route_key);
        self.magpie_effort_generation += 1;
        let generation = self.magpie_effort_generation;
        self.magpie_effort = None;
        if self.preview_state.is_some() || crate::harness::screenshot_mode()
            || crate::harness::remote_host(&self.session_id).is_some()
        {
            self.magpie_effort = Some(MagpieEffort::Unknown);
            return;
        }
        let work = cx.background_executor().spawn(async move {
            crate::custom_providers::custom_provider_effort(&profile, &model)
        });
        cx.spawn(async move |this, cx| {
            let capability = work.await;
            let _ = this.update(cx, |panel, cx| {
                if panel.uses_custom_provider_effort()
                    && panel.magpie_effort_generation == generation
                {
                    panel.magpie_effort = Some(capability);
                    let levels = panel.effort_ladder();
                    let selected = levels.iter().copied()
                        .find(|level| panel.reasoning_effort.as_deref() == Some(*level))
                        .or_else(|| levels.first().copied());
                    if let Some(picker) = &mut panel.effort_picker {
                        picker.selected = selected;
                    }
                    panel.sync_effort_menu(cx);
                    cx.notify();
                }
            });
        }).detach();
    }

    fn custom_provider_default_effort(&self) -> Option<&str> {
        match self.magpie_effort.as_ref() {
            Some(MagpieEffort::Known { default, .. }) => default.as_deref(),
            _ => None,
        }
    }

    pub(super) fn effort_pill_label(&self) -> Option<String> {
        self.model.as_ref()?;
        if let Some(level) = self.reasoning_effort.as_deref().filter(|level| super::swarm::is_swarm_effort(level)) {
            return Some(super::swarm::effort_label(level).into());
        }
        if self.uses_custom_provider_effort() {
            return Some(match self.magpie_effort.as_ref() {
                Some(MagpieEffort::Unsupported) => tr("No reasoning", "无推理").into(),
                Some(MagpieEffort::Known { fixed: true, default, .. }) => {
                    match default.as_deref() {
                        Some(level) => format!("{} · {}", tr("Fixed", "固定"), effort_level_label(level)),
                        None => tr("Fixed", "固定").into(),
                    }
                }
                _ => self.reasoning_effort.as_deref()
                    .map(|level| effort_level_label(level).to_string())
                    .unwrap_or_else(|| tr("Effort", "推理强度").into()),
            });
        }
        self.reasoning_effort.clone().or_else(|| {
            (!self.effort_ladder().is_empty()).then(|| tr("Effort", "推理强度").into())
        })
    }

    pub(super) fn effort_unavailable_message(&self) -> String {
        if !self.uses_custom_provider_effort() {
            return tr("This model offers no adjustable reasoning effort.", "此模型没有可调节的推理强度。").into();
        }
        match self.magpie_effort.as_ref() {
            None => tr("Reading the endpoint’s model catalog…", "正在读取端点模型目录…").into(),
            Some(MagpieEffort::Unknown) => tr("No effort levels were available from the endpoint catalog.", "未从端点目录取得推理档位。").into(),
            Some(MagpieEffort::Unsupported) => tr("Reasoning is disabled for this route.", "此路由未启用推理。").into(),
            Some(MagpieEffort::RuntimeLimited) => tr("This Jcode runtime cannot apply the advertised levels.", "当前 Jcode 运行时无法应用网关声明的档位。").into(),
            Some(MagpieEffort::Known { default, .. }) => match default.as_deref() {
                Some(level) => format!("{} · {}", tr("Fixed gateway level", "网关固定档位"), effort_level_label(level)),
                None => tr("Fixed gateway level", "网关固定档位").into(),
            },
        }
    }

    /// Keep the composer's `/effort` menu on the serving model's ladder.
    /// Deferred: `/effort` runs inside the composer's own submit callback.
    pub(super) fn sync_effort_menu(&self, cx: &mut Context<Self>) {
        let ladder = self.effort_ladder();
        let current = self.reasoning_effort.clone();
        let input = self.input.clone();
        cx.defer(move |cx| {
            input.update(cx, |input, cx| input.set_effort_state(ladder, current, cx))
        });
    }

    /// Validate and send one effort change. Returns false, after reporting
    /// why in the transcript, when the level is not offered.
    pub(super) fn request_effort(&mut self, level: &str, cx: &mut Context<Self>) -> bool {
        if self.is_side_document() || self.preview_state.is_some() {
            return false;
        }
        let ladder = self.effort_ladder();
        if ladder.is_empty() {
            let notice = self.effort_unavailable_message();
            self.items.push(Item::Error(notice));
            return false;
        }
        let Some(level) = ladder.iter().copied().find(|candidate| *candidate == level) else {
            self.items.push(Item::Error(format!(
                "Usage: `/effort <{}>`",
                ladder.join("|")
            )));
            return false;
        };
        self.send_effort(level, cx);
        true
    }

    pub(super) fn send_effort(&mut self, level: &str, cx: &mut Context<Self>) {
        if let Some(previous) = self.reasoning_effort.as_ref().filter(|level| !super::swarm::is_swarm_effort(level)) {
            self.last_standard_effort = Some(previous.clone());
        }
        let pending = self.pending_effort.get_or_insert_with(|| PendingEffort {
            previous: self.reasoning_effort.clone(),
            requested: VecDeque::new(),
        });
        pending.requested.push_back(level.to_string());
        self.reasoning_effort = Some(level.to_string());
        self.bridge.send(Command::SessionOperation {
            session_id: self.session_id.clone(),
            operation: SessionOperation::SetEffort(level.to_string()),
        });
        self.sync_effort_menu(cx);
        cx.notify();
    }

    /// One rung up (`direction > 0`) or down, from the fast effort keys.
    pub(super) fn step_effort(&mut self, direction: i8, cx: &mut Context<Self>) {
        let ladder = self.effort_ladder();
        if ladder.is_empty() {
            self.effort_notice(self.effort_unavailable_message(), cx);
            return;
        }
        match effort::step(&ladder, self.reasoning_effort.as_deref(), direction) {
            Some(next) => {
                if self.request_effort(next, cx) {
                    self.effort_notice(format!("Effort: {}", effort::label(next)), cx);
                }
            }
            None => {
                let current = self
                    .reasoning_effort
                    .as_deref()
                    .map(effort::label)
                    .unwrap_or("default");
                let end = if direction > 0 {
                    "strongest"
                } else {
                    "weakest"
                };
                self.effort_notice(format!("Effort: {current} (already {end})"), cx);
            }
        }
    }

    /// A quiet confirmation under the composer, like the TUI status notice.
    fn effort_notice(&mut self, notice: String, cx: &mut Context<Self>) {
        let input = self.input.clone();
        cx.defer(move |cx| input.update(cx, |input, cx| input.set_transient_notice(notice, cx)));
    }

    /// The runtime answered one effort request.
    pub(crate) fn effort_settled(
        &mut self,
        effort: &str,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_effort.as_mut() else {
            if let Some(error) = error {
                self.items.push(Item::Error(format!(
                    "Failed to set effort `{effort}`: {error}"
                )));
            }
            cx.notify();
            return;
        };
        let Some(index) = pending.requested.iter().position(|level| level == effort) else {
            return;
        };
        pending.requested.remove(index);
        let settled = pending.requested.is_empty();
        if let Some(error) = error {
            self.items.push(Item::Error(format!(
                "Failed to set effort `{effort}`: {error}"
            )));
            crate::sounds::play(crate::sounds::Cue::Error, cx);
            if settled {
                // Nothing newer is in flight, so the pill must return to the
                // level the provider is still using.
                self.reasoning_effort = pending.previous.clone();
            }
        }
        if settled {
            self.pending_effort = None;
        }
        self.sync_effort_menu(cx);
        cx.notify();
    }

    /// Identity reported an effort. While requests are in flight, an older
    /// broadcast must not snap the pill back between rapid keypresses.
    pub(super) fn identity_effort(&mut self, reported: Option<String>) {
        if let Some(level) = reported.as_ref().filter(|level| !super::swarm::is_swarm_effort(level)) {
            self.last_standard_effort = Some(level.clone());
        }
        match &mut self.pending_effort {
            Some(pending) => {
                // Remember the runtime's truth for a later refusal, but keep
                // showing the newest request.
                pending.previous = reported;
            }
            None => self.reasoning_effort = reported,
        }
    }
}

fn effort_level_label(level: &str) -> &str {
    if crate::locale::is_zh() {
        match level {
            "none" => "关闭",
            "minimal" => "极低",
            "low" => "低",
            "medium" => "中",
            "high" => "高",
            "xhigh" => "极高",
            "max" => "最高",
            "swarm" => "多代理",
            "swarm-deep" => "深度多代理",
            _ => level,
        }
    } else {
        effort::label(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Command;
    use std::sync::mpsc::Receiver;

    fn setup<'a>(
        cx: &'a mut gpui::TestAppContext,
        provider: &str,
        model: &str,
        effort: Option<&str>,
    ) -> (
        Entity<Panel>,
        &'a mut gpui::VisualTestContext,
        Receiver<Command>,
    ) {
        cx.update(crate::input::bind_keys);
        let (bridge, commands) = crate::harness::spawn_recording();
        let (workspace, vcx) = cx.add_window_view(|_, cx| {
            let mut workspace =
                crate::workspace::Workspace::for_test(crate::learning::Coach::new(), cx);
            workspace.set_test_bridge(bridge);
            workspace.push_test_panel("session-a", cx);
            workspace
        });
        let mut panel = None;
        workspace.update(vcx, |workspace, _| panel = workspace.test_panel(0));
        let panel = panel.expect("test panel exists");
        panel.update(vcx, |panel, cx| {
            panel.apply(
                &ApiEvent::ModelInfo {
                    session_id: "session-a".into(),
                    provider: Some(provider.into()),
                    model: Some(model.into()),
                    reasoning_effort: effort.map(str::to_string),
                    auth_method: None,
                },
                cx,
            );
        });
        vcx.update(|window, cx| {
            let handle = panel.read(cx).input.read(cx).focus_handle.clone();
            window.focus(&handle, cx);
        });
        vcx.run_until_parked();
        (panel, vcx, commands)
    }

    fn sent_efforts(commands: &Receiver<Command>) -> Vec<String> {
        commands
            .try_iter()
            .filter_map(|command| match command {
                Command::SessionOperation {
                    operation: SessionOperation::SetEffort(effort),
                    ..
                } => Some(effort),
                _ => None,
            })
            .collect()
    }

    fn effort_of(panel: &Entity<Panel>, vcx: &mut gpui::VisualTestContext) -> Option<String> {
        panel.read_with(vcx, |panel, _| panel.reasoning_effort.clone())
    }

    /// The configured chords (Alt+Left/Right by default) step one rung at a
    /// time from the focused composer, stop at the ends, and update the pill
    /// immediately without waiting for the runtime.
    #[gpui::test]
    fn fast_effort_keys_step_the_ladder_from_the_composer(cx: &mut gpui::TestAppContext) {
        let keys = crate::effort::EffortKeys::from_config();
        let (Some(up), Some(down)) = (keys.increase.first(), keys.decrease.first()) else {
            return; // Effort keys disabled in this user's config.
        };
        let (panel, vcx, commands) = setup(cx, "openai", "gpt-5.6-sol", Some("high"));

        vcx.simulate_keystrokes(up);
        vcx.run_until_parked();
        assert_eq!(effort_of(&panel, vcx).as_deref(), Some("xhigh"));
        vcx.simulate_keystrokes(up);
        vcx.simulate_keystrokes(down);
        vcx.simulate_keystrokes(down);
        vcx.run_until_parked();
        assert_eq!(effort_of(&panel, vcx).as_deref(), Some("high"));
        assert_eq!(sent_efforts(&commands), ["xhigh", "max", "xhigh", "high"]);
        assert_eq!(
            panel.read_with(vcx, |panel, cx| panel
                .input
                .read(cx)
                .transient_notice()
                .map(str::to_string)),
            Some("Effort: High".into())
        );
        assert_eq!(
            panel.read_with(vcx, |panel, cx| panel.input.read(cx).content.to_string()),
            "",
            "effort keys must not type or move text in the composer"
        );
    }

    #[gpui::test]
    fn fast_effort_keys_stop_at_the_strongest_level(cx: &mut gpui::TestAppContext) {
        let keys = crate::effort::EffortKeys::from_config();
        let Some(up) = keys.increase.first() else {
            return;
        };
        let (panel, vcx, commands) = setup(cx, "anthropic", "claude-sonnet-4-6", None);
        let top = *panel
            .read_with(vcx, |panel, _| panel.effort_ladder())
            .last()
            .unwrap();
        panel.update(vcx, |panel, _| panel.reasoning_effort = Some(top.into()));
        vcx.simulate_keystrokes(up);
        vcx.run_until_parked();
        assert_eq!(effort_of(&panel, vcx).as_deref(), Some(top));
        assert!(
            sent_efforts(&commands).is_empty(),
            "nothing to send at the top"
        );
        assert!(
            panel
                .read_with(vcx, |panel, cx| panel
                    .input
                    .read(cx)
                    .transient_notice()
                    .map(str::to_string))
                .is_some_and(|notice| notice.contains("already strongest"))
        );
    }

    /// A refused request restores the level the provider is still using, and
    /// an identity broadcast between rapid presses does not snap the pill back.
    #[gpui::test]
    fn a_refusal_restores_the_previous_effort(cx: &mut gpui::TestAppContext) {
        let (panel, vcx, commands) = setup(cx, "openai", "gpt-5.6-sol", Some("medium"));
        panel.update(vcx, |panel, cx| {
            assert!(panel.request_effort("high", cx));
            assert!(panel.request_effort("xhigh", cx));
            // The first change's broadcast arrives while xhigh is in flight.
            panel.apply(
                &ApiEvent::ModelInfo {
                    session_id: "session-a".into(),
                    provider: Some("openai".into()),
                    model: Some("gpt-5.6-sol".into()),
                    reasoning_effort: Some("high".into()),
                    auth_method: None,
                },
                cx,
            );
            assert_eq!(panel.reasoning_effort.as_deref(), Some("xhigh"));
            panel.effort_settled("high", None, cx);
            panel.effort_settled("xhigh", Some("not supported"), cx);
            assert_eq!(panel.reasoning_effort.as_deref(), Some("high"));
            assert!(matches!(panel.items.last(), Some(Item::Error(error)) if error.contains("not supported")));
            // Settled: later identity is authoritative again.
            panel.apply(
                &ApiEvent::ModelInfo {
                    session_id: "session-a".into(),
                    provider: Some("openai".into()),
                    model: Some("gpt-5.6-sol".into()),
                    reasoning_effort: None,
                    auth_method: None,
                },
                cx,
            );
            assert_eq!(panel.reasoning_effort, None);
        });
        assert_eq!(sent_efforts(&commands), ["high", "xhigh"]);
    }

    /// `/effort` offers and accepts only the serving model's levels.
    #[gpui::test]
    fn slash_effort_follows_the_serving_model(cx: &mut gpui::TestAppContext) {
        let (panel, vcx, commands) = setup(cx, "anthropic", "claude-sonnet-4-6", Some("high"));
        vcx.simulate_input("/effort minimal");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(
            sent_efforts(&commands).is_empty(),
            "Claude has no minimal level"
        );
        assert!(panel.read_with(vcx, |panel, _| matches!(
            panel.items.last(),
            Some(Item::Error(error)) if error.contains("low|medium|high")
        )));

        panel.update(vcx, |panel, cx| {
            panel
                .input
                .update(cx, |input, cx| input.set_content(String::new(), cx))
        });
        vcx.simulate_input("/effort Low");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(sent_efforts(&commands), ["low"]);
        assert_eq!(effort_of(&panel, vcx).as_deref(), Some("low"));
    }

    #[gpui::test]
    fn effort_menu_selects_a_level_without_editing_the_draft(cx: &mut gpui::TestAppContext) {
        let (panel, vcx, commands) = setup(cx, "openai", "gpt-5.6-sol", Some("xhigh"));
        vcx.simulate_input("keep this draft");
        let click_pill = |vcx: &mut gpui::VisualTestContext| {
            let pill = vcx.debug_bounds("panel-model-effort").expect("effort pill");
            vcx.simulate_click(pill.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
        };
        let assert_draft = |vcx: &mut gpui::VisualTestContext| {
            panel.read_with(vcx, |panel, cx| {
                assert_eq!(panel.input.read(cx).content.as_ref(), "keep this draft");
            });
        };

        click_pill(vcx);
        assert_draft(vcx);
        let high = vcx.debug_bounds("effort-level-high").expect("independent effort row");
        vcx.simulate_click(high.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert_eq!(sent_efforts(&commands), ["high"]);
        assert!(vcx.debug_bounds("effort-picker").is_none());
        assert_draft(vcx);

        click_pill(vcx);
        vcx.simulate_keystrokes("down enter");
        vcx.run_until_parked();
        assert_eq!(sent_efforts(&commands), ["xhigh"]);
        assert!(vcx.debug_bounds("effort-picker").is_none());
        assert_draft(vcx);

        click_pill(vcx);
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(sent_efforts(&commands).is_empty());
        assert!(vcx.debug_bounds("effort-picker").is_none());
        assert_draft(vcx);
    }
}
