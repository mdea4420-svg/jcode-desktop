//! Composer chrome: the input with one row of compact pills below it.
//!
//! Nothing sits above the input. The bottom row holds the model pill, a
//! separate reasoning effort pill, the credential method pill and the context
//! ring on the left, and the publish button plus the location (repo or
//! directory) on the right. The voice button lives inside the input box.
//! Pills are detached from the input and shaped like the transcript's user
//! prompt cards, so nothing reads as a folder tab.
use super::*;

/// Pill height.
pub(super) const TAB_HEIGHT: f32 = 22.;
/// Gap between the input and the pill row below it.
const PILL_GAP: f32 = 4.;
/// Distance from the input's right edge to the in-box voice button.
const VOICE_INSET: f32 = 8.;
/// Height of the soft ramp above the floating composer.
const DOCK_FADE: f32 = 28.;
/// Opacity of the veil behind the composer. High enough that history behind
/// the controls is a quiet hint, never competing with the draft.
const DOCK_VEIL_ALPHA: f32 = 0.86;
/// Right padding for the in-box voice button and its inset.
pub(super) const VOICE_TRAILING_SPACE: f32 = 44.;

pub(super) fn composer_picker_max_height(window: &Window) -> gpui::Pixels {
    // Leave space for the title bar, composer, and its pill row.
    (window.viewport_size().height - px(168.)).clamp(px(136.), px(440.))
}

impl Panel {
    /// Prompt input with its pill row below. Every composer location
    /// (fresh session, startup layout, docked) uses this so the pills never
    /// drift apart from the input they label.
    pub(super) fn render_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let _ = window;
        let theme = Theme::global();
        let screenshot_mode = crate::config::display().screenshot_mode;
        let route_channel = self.current_route_channel(cx);
        let account_label = if screenshot_mode {
            "Account hidden".to_string()
        } else {
            route_method_label(self.provider.as_deref(), self.auth_method.as_deref(), route_channel)
        };
        let model_label = route_channel
            .and(self.model.as_deref())
            .map(crate::input::model_menu::pretty_model_without_channel)
            .unwrap_or_else(|| pretty_model_label(self.model.as_deref()));
        let limits = (!screenshot_mode)
            .then(|| self.limits_summary(cx))
            .flatten();
        let effort = self.effort_pill_label();
        let remote_machine = (!screenshot_mode)
            .then(|| crate::harness::remote_host(&self.session_id))
            .flatten()
            .map(|host| {
                if host == crate::managed_cloud::HOST {
                    "Jcode Cloud".to_string()
                } else {
                    host
                }
            });
        let location = self
            .working_dir
            .as_deref()
            .filter(|dir| !screenshot_mode && !dir.is_empty())
            .map(|dir| {
                let (name, detail) = if remote_machine.is_some() {
                    remote_location_label(dir)
                } else {
                    location_label(dir)
                };
                (name, detail, dir.to_owned())
            });
        // Build info, usage limits and status share the pill row, so the
        // panel needs no separate footer bar below the composer.
        let status_line = self.status_line();
        let meta = div()
            .debug_selector(|| "panel-meta".into())
            .flex_shrink(4.)
            .min_w_0()
            .h(px(TAB_HEIGHT))
            .px_1()
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .flex_nowrap()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(10.0))
            .font_family(theme.FONT_MONO)
            .text_color(theme.TEXT_FAINT)
            .child(
                div()
                    .debug_selector(|| "panel-status".into())
                    .min_w_0()
                    .flex_shrink_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    .children(self.render_voice_status(status_line))
                    .children(self.render_usage_meters())
                    .children(self.render_image_pane_toggle(cx)),
            )
            .when(self.show_build_footer && !screenshot_mode, |el| {
                el.child(
                    div()
                        .id("panel-build")
                        .debug_selector(|| "panel-build".into())
                        .tooltip(|_, cx| cx.new(|_| crate::build_info::BuildTooltip).into())
                        // Build metadata yields space to the identity pills.
                        .flex_shrink_1()
                        .min_w_0()
                        .truncate()
                        .child(crate::build_info::label()),
                )
            });
        let pills = div()
            .debug_selector(|| "composer-tabs".into())
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .mt(px(PILL_GAP))
            .child(
                div()
                    .debug_selector(|| "panel-identity".into())
                    // Auto basis: the pills claim their width before the
                    // trailing directory, which truncates first.
                    .flex_auto()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .child(
                        composer_pill("panel-model")
                            .flex_shrink_1()
                            .min_w(px(56.))
                            .child(
                                div()
                                    .debug_selector(|| "panel-model-name".into())
                                    .min_w_0()
                                    .truncate()
                                    .child(model_label),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_model_picker(window, cx);
                                cx.stop_propagation();
                            })),
                    )
                    .children(effort.map(|effort| {
                        composer_pill("panel-model-effort")
                            .relative()
                            .flex_shrink(3.)
                            .min_w(px(24.))
                            .text_color(theme.TEXT_FAINT)
                            .child(div().min_w_0().truncate().child(effort))
                            .children(self.effort_pill_bounds_marker())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_effort_picker(window, cx);
                                cx.stop_propagation();
                            }))
                    }))
                    .child(
                        composer_pill("panel-login")
                            .flex_shrink(2.)
                            .min_w(px(48.))
                            .text_color(theme.TEXT_FAINT)
                            .child(div().min_w_0().truncate().child(account_label.clone()))
                            .when_some(limits, |pill, summary| {
                                pill.tooltip(move |_, cx| {
                                    let method = account_label.clone();
                                    let summary = summary.clone();
                                    cx.new(|_| super::usage::LimitsTooltip { method, summary })
                                        .into()
                                })
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_model_routes(window, cx);
                                cx.stop_propagation();
                            })),
                    )
                    .children(self.render_context_meter(cx))
                    .children(self.render_swarm_pill(cx)),
            )
            .child(
                div()
                    .debug_selector(|| "composer-trailing".into())
                    // The directory yields space before the identity pills.
                    .flex_shrink(8.)
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .overflow_hidden()
                    .child(meta)
                    .children(self.render_publish_button(cx))
                    .children(remote_machine.map(|machine| {
                        // Remote sessions name their machine first, so a cloud
                        // chat never reads like a local one.
                        div()
                            .id("composer-machine")
                            .debug_selector(|| "composer-machine".into())
                            .flex_none()
                            .h(px(TAB_HEIGHT))
                            .px_2p5()
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded_full()
                            .bg(theme.ACCENT_DIM)
                            .text_color(theme.ACCENT)
                            .text_size(px(10.5))
                            .font_family(theme.FONT_MONO)
                            .whitespace_nowrap()
                            .child(machine)
                    }))
                    .children(location.map(|(name, detail, path)| {
                        let hint = crate::locale::tr("Click to copy full path", "点击复制完整路径");
                        let tooltip = format!("{path}\n{hint}");
                        div()
                            .id("composer-location")
                            .debug_selector(|| "composer-location".into())
                            .flex_shrink_1()
                            .min_w_0()
                            .px_1()
                            .flex()
                            .items_baseline()
                            .gap_1p5()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(11.5))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|el| el.bg(theme.USER_BG))
                            .tooltip(move |_, cx| {
                                cx.new(|_| super::usage::MeterTooltip(tooltip.clone())).into()
                            })
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()));
                                cx.stop_propagation();
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(theme.TEXT_DIM)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(name),
                            )
                            .children(detail.map(|detail| {
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(10.5))
                                    .font_family(theme.FONT_MONO)
                                    .text_color(theme.TEXT_FAINT)
                                    .child(detail)
                            }))
                    })),
            );
        let strips = self.render_composer_applets(window, cx);
        div()
            .debug_selector(|| "composer".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .children(strips)
            .children(self.render_effort_picker(window, cx))
            .children(self.render_swarm_picker(window, cx))
            .child(
                // The voice button floats inside the input's right edge. The
                // input reserves matching trailing space for it.
                div()
                    .relative()
                    .w_full()
                    .min_w_0()
                    .child(self.render_voice_input_slot(cx))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(VOICE_INSET))
                            .flex()
                            .items_center()
                            .child(self.render_voice_tab(cx)),
                    ),
            )
            .child(pills)
            .into_any_element()
    }

    pub(super) fn render_project_folder_actions(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::{locale::tr, ui_actions::ToggleProjectFolderMenu};

        let theme = Theme::global();
        div()
            .px_1()
            .flex()
            .child(
                div()
                    .id("composer-project-folder")
                    .debug_selector(|| "composer-project-folder".into())
                    .h(px(28.0))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.PANEL_BORDER)
                    .bg(theme.ACCENT_DIM)
                    .font_family(theme.FONT_UI)
                    .text_size(px(11.0))
                    .text_color(theme.TEXT)
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover(|el| el.border_color(theme.ACCENT).bg(theme.TOOL_BG))
                    .active(|el| el.bg(theme.USER_BG))
                    .max_w_full()
                    .min_w_0()
                    .child(div().min_w_0().truncate()
                        .when(cfg!(test), |label| {
                            let panel = cx.entity_id();
                            label.debug_selector(move || format!("composer-project-folder-label-{panel:?}"))
                        })
                        .child(match &self.project_folder_label {
                            Some(name) => format!("{} · {name}", tr("Project", "项目")),
                            None => tr("Project", "项目").to_owned(),
                        }))
                    .child(crate::ui_actions::project_folder_plus_icon())
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.focus_input(window, cx);
                        let action = ToggleProjectFolderMenu {
                            source: Some(cx.entity_id()),
                            position: event.position,
                        };
                        // Activating the source slot reads this panel after its click update ends.
                        window.defer(cx, move |window, cx| window.dispatch_action(Box::new(action), cx));
                    })),
            )
            .into_any_element()
    }

    /// The ordinary composer, floating over the bottom of the transcript.
    /// History scrolls beneath a translucent veil instead of stopping at a
    /// hard edge. The veil eases from clear to mostly opaque so text behind
    /// the controls reads as a soft, frosted hint rather than competing ink.
    /// Its measured height pads the list so the live end clears it.
    pub(super) fn render_composer_dock(
        &mut self,
        input_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let background = Theme::global().panel_background(self.surface_focused);
        let veil = |alpha: f32| gpui::Rgba {
            a: alpha,
            ..background
        };
        let panel = cx.entity().downgrade();
        div()
            .debug_selector(|| "composer-dock".into())
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .flex_col()
            // Do not let clicks reach transcript rows hidden under the veil,
            // but keep wheel and touchpad scrolling the transcript.
            .block_mouse_except_scroll()
            .child(
                div()
                    .debug_selector(|| "composer-veil".into())
                    .w_full()
                    .h(px(DOCK_FADE))
                    .bg(gpui::linear_gradient(
                        0.,
                        gpui::linear_color_stop(veil(DOCK_VEIL_ALPHA), 0.),
                        gpui::linear_color_stop(veil(0.), 1.),
                    )),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .px_2()
                    .pb_2()
                    .bg(veil(DOCK_VEIL_ALPHA))
                    .when(self.recovery_picker_open, |el| {
                        el.child(self.render_recovery_model_picker(cx))
                    })
                    .children(self.render_voice_overlay(window, cx))
                    .children(self.render_preview_badge(cx))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(self.render_prompt_queue(cx))
                            .children(self.render_voice_pending())
                            .child(self.render_composer(window, cx))
                            .child(startup::input_marker(input_bounds)),
                    ),
            )
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, _, cx| {
                        // Measure after paint. The fade is see-through, so
                        // only the solid part below it reserves list space.
                        let height = (f32::from(bounds.size.height) - DOCK_FADE).max(0.).round();
                        let panel = panel.clone();
                        cx.defer(move |cx| {
                            let Some(entity) = panel.upgrade() else {
                                return;
                            };
                            if (entity.read(cx).composer_dock_height - height).abs() < 0.5 {
                                return;
                            }
                            entity.update(cx, |panel, cx| {
                                panel.composer_dock_height = height;
                                cx.notify();
                            });
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    /// Composer-placed applet instances for this session, as a compact strip
    /// directly above the pills (suggestions, pickers).
    fn render_composer_applets(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let instances: Vec<String> = crate::applet_runtime::get(cx)
            .host
            .borrow()
            .composer_for(&self.session_id)
            .map(|mounted| mounted.instance.id.clone())
            .collect();
        if instances.is_empty() {
            return None;
        }
        let selection = self.applet_selection.clone();
        let cards: Vec<gpui::AnyElement> = instances
            .iter()
            .filter_map(|instance| {
                crate::applet_surface::card(instance, false, &selection, window, cx)
            })
            .collect();
        Some(
            div()
                .debug_selector(|| "composer-applets".into())
                .w_full()
                .min_w_0()
                .px_1()
                .mb(px(PILL_GAP))
                .flex()
                .flex_col()
                .gap_1()
                .children(cards)
                .into_any_element(),
        )
    }
}

/// A compact rounded pill, filled like the transcript's user prompt cards.
pub(super) fn composer_pill(id: &'static str) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(TAB_HEIGHT))
        .px_2p5()
        .flex()
        .items_center()
        .gap_1p5()
        .rounded_full()
        .bg(theme.prompt_background(usize::MAX))
        .text_size(px(10.5))
        .font_family(theme.FONT_MONO)
        .text_color(theme.TEXT_DIM)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|el| el.text_color(theme.TEXT).bg(theme.USER_BG))
}

/// `claude-opus-4-8` reads `Opus 4.8`. The provider already appears in the
/// method pill, so the redundant `Claude` family prefix is dropped, matching
/// the TUI's compact model label.
pub(super) fn pretty_model_label(model: Option<&str>) -> String {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return crate::locale::tr("Choose model", "选择模型").into();
    };
    let pretty = jcode_provider_core::model_names::pretty_model_display_name(model);
    match pretty.strip_prefix("Claude ") {
        Some(rest) if !rest.trim().is_empty() => rest.to_string(),
        _ => pretty,
    }
}

/// Model label plus effort, used where a single string is needed.
#[cfg(test)]
pub(super) fn model_tab_label(model: Option<&str>, effort: Option<&str>) -> String {
    let pretty = pretty_model_label(model);
    match effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        Some(effort) if model.is_some_and(|m| !m.trim().is_empty()) => format!("{pretty} {effort}"),
        _ => pretty,
    }
}

/// Repo name (the last path component) plus its home-relative parent path.
/// `/home/me/src/jcode` reads `jcode` then `~/src`. Windows paths split on
/// either separator, so `C:\Users\me\src\jcode` reads `jcode` then `~\src`.
pub(super) fn location_label(path: &str) -> (String, Option<String>) {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return ("/".into(), None);
    }
    if crate::platform_dirs::tilde(trimmed) == "~" {
        return ("~".into(), None);
    }
    let (parent, name) = match trimmed.rsplit_once(['/', '\\']) {
        Some((parent, name)) => (parent, name),
        None => return (trimmed.into(), None),
    };
    // `C:\` keeps its separator so the parent of `C:\repo` is not `C:`.
    let parent = if parent.is_empty() {
        "/".to_string()
    } else if parent.ends_with(':') {
        trimmed[..parent.len() + 1].to_string()
    } else {
        crate::platform_dirs::tilde(parent)
    };
    (name.to_string(), Some(parent))
}

/// Repo name plus its full parent path on another machine. The local `$HOME`
/// says nothing about the remote user's home, so nothing is abbreviated
/// except the managed cloud's own home directory.
pub(super) fn remote_location_label(path: &str) -> (String, Option<String>) {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return ("/".into(), None);
    }
    let cloud_home = "/home/ec2-user";
    if trimmed == cloud_home {
        return ("~".into(), None);
    }
    let Some((parent, name)) = trimmed.rsplit_once('/') else {
        return (trimmed.into(), None);
    };
    let parent = if parent.is_empty() { "/" } else { parent };
    let parent = match parent.strip_prefix(cloud_home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => parent.to_string(),
    };
    (name.to_string(), Some(parent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_tab_uses_pretty_names_and_effort() {
        assert_eq!(model_tab_label(None, Some("high")), "Choose model");
        assert_eq!(model_tab_label(Some(" "), None), "Choose model");
        assert_eq!(model_tab_label(Some("claude-opus-4-8"), None), "Opus 4.8");
        assert_eq!(
            model_tab_label(Some("gpt-5.5"), Some("high")),
            "GPT-5.5 high"
        );
        assert_eq!(model_tab_label(None, Some("high")), "Choose model");
        assert_eq!(
            model_tab_label(Some("gpt-5.1-codex-max"), Some("")),
            "GPT-5.1 Codex Max"
        );
    }

    #[test]
    fn remote_location_keeps_the_remote_path() {
        assert_eq!(
            remote_location_label("/home/ec2-user/workspaces"),
            ("workspaces".into(), Some("~".into()))
        );
        assert_eq!(
            remote_location_label("/home/ec2-user/workspaces/app/"),
            ("app".into(), Some("~/workspaces".into()))
        );
        assert_eq!(
            remote_location_label("/srv/app"),
            ("app".into(), Some("/srv".into()))
        );
        assert_eq!(remote_location_label("/home/ec2-user"), ("~".into(), None));
        assert_eq!(
            remote_location_label("/home/ec2-userx/a"),
            ("a".into(), Some("/home/ec2-userx".into()))
        );
    }

    #[test]
    fn location_label_splits_repo_name_from_parent() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            location_label(&format!("{home}/jcode")),
            ("jcode".into(), Some("~".into()))
        );
        assert_eq!(
            location_label(&format!("{home}/src/jcode/")),
            ("jcode".into(), Some("~/src".into()))
        );
        assert_eq!(location_label(&home), ("~".into(), None));
        assert_eq!(
            location_label("/srv/app"),
            ("app".into(), Some("/srv".into()))
        );
        assert_eq!(location_label("/"), ("/".into(), None));
        assert_eq!(
            location_label(r"D:\work\jcode\"),
            ("jcode".into(), Some(r"D:\work".into()))
        );
        assert_eq!(
            location_label(r"C:\repo"),
            ("repo".into(), Some(r"C:\".into()))
        );
    }

    #[gpui::test]
    fn pills_sit_below_the_input_with_location_on_the_right(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        let handle = vcx.update(|window, _| window.window_handle());
        for width in [240., 480., 1440.] {
            vcx.simulate_window_resize(handle, gpui::size(px(width), px(600.)));
            panel.update(vcx, |panel, cx| {
                panel.model = Some("claude-opus-4-8".into());
                panel.reasoning_effort = Some("high".into());
                panel.provider = Some("anthropic".into());
                panel.auth_method = Some("oauth".into());
                panel.working_dir = Some("/srv/projects/jcode".into());
                *panel.items = vec![Item::User("Hello".into())];
                cx.notify();
            });
            vcx.run_until_parked();
            let input = vcx.debug_bounds("prompt-input").unwrap();
            let editor = vcx.debug_bounds("prompt-editor").unwrap();
            let model = vcx.debug_bounds("panel-model").unwrap();
            let name = vcx.debug_bounds("panel-model-name").unwrap();
            let effort = vcx.debug_bounds("panel-model-effort").unwrap();
            let login = vcx.debug_bounds("panel-login").unwrap();
            let context = vcx.debug_bounds("panel-context-meter").unwrap();
            let voice = vcx.debug_bounds("voice-toggle").unwrap();
            for (tab_name, tab) in [("model", model), ("login", login), ("context", context)] {
                assert!(
                    tab.top() > input.bottom(),
                    "{tab_name} pill sits below the input at {width}: {tab:?} {input:?}"
                );
                assert!(tab.left() >= input.left() && tab.right() <= input.right());
            }
            assert!(name.right() <= model.right());
            assert!(model.right() <= effort.left(), "effort follows the model");
            assert!(effort.right() <= login.left(), "method follows effort");
            assert!(
                login.right() <= context.left(),
                "context ring follows method"
            );
            // The voice button lives inside the input box, on its right, and
            // the editor never runs underneath it.
            assert!(
                voice.left() >= input.left()
                    && voice.right() <= input.right()
                    && voice.top() >= input.top()
                    && voice.bottom() <= input.bottom(),
                "voice sits inside the input at {width}: {voice:?} {input:?}"
            );
            assert!(editor.right() <= voice.left(), "{editor:?} {voice:?}");
            if width >= 480. {
                let location = vcx.debug_bounds("composer-location").unwrap();
                assert!(
                    location.top() > input.bottom(),
                    "location is below the input"
                );
                assert!(context.right() <= location.left(), "location sits right");
                assert!(
                    input.right() - location.right() < px(12.),
                    "location hugs the right edge at {width}: {location:?} {input:?}"
                );
            }
            if width == 1440. {
                let location = vcx.debug_bounds("composer-location").unwrap();
                vcx.simulate_click(location.center(), gpui::Modifiers::default());
                let copied = vcx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text());
                assert_eq!(copied.as_deref(), Some("/srv/projects/jcode"));
            }
        }
    }
}
