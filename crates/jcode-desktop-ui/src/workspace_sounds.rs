//! App-wide opt-in sound controls. The setting is persisted, not a window snapshot.
use super::*;
use crate::sounds::{self, Cue};

impl Workspace {
    pub(super) fn render_sound_settings(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let enabled = sounds::enabled(cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .mt_2()
            .child(div().text_color(Theme::global().TEXT_DIM).child(crate::locale::tr("Sounds · all windows", "提示音 · 全部窗口")))
            .child(
                div()
                    .id("settings-sounds")
                    .debug_selector(|| "settings-sounds".into())
                    .px_3()
                    .py_2()
                    .rounded_full()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .bg(Theme::global().PANEL_BG)
                    .hover(|el| el.bg(Theme::global().TOOL_BG))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        let enabled = !sounds::enabled(cx);
                        if let Err(error) = crate::config::persist_sounds_enabled(enabled) {
                            this.status = format!("Could not save sound setting: {error}");
                            cx.notify();
                            return;
                        }
                        sounds::set_enabled(enabled, cx);
                        if enabled {
                            sounds::play(Cue::Complete, cx);
                        }
                        cx.notify();
                    }))
                    .child(crate::locale::tr("Sound effects", "界面提示音效"))
                    .child(div().text_color(if enabled { Theme::global().ACCENT } else { Theme::global().TEXT_DIM }).child(if enabled { crate::locale::tr("On", "开启") } else { crate::locale::tr("Off", "关闭") })),
            )
            .child(div().text_color(Theme::global().TEXT_DIM).child(crate::locale::tr(
                "Soft cues for messages, attention, errors, finished tasks, and opening or closing panels. Off by default.",
                "为新消息、提醒、错误、任务完成及开关面板播放柔和提示音。默认关闭。",
            )))
            .when(enabled, |el| el.child(
                div()
                    .id("settings-sounds-preview")
                    .debug_selector(|| "settings-sounds-preview".into())
                    .px_3()
                    .py_2()
                    .rounded_full()
                    .cursor_pointer()
                    .bg(Theme::global().PANEL_BG)
                    .hover(|el| el.bg(Theme::global().TOOL_BG))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|_, _, _, cx| {
                        sounds::play(Cue::Complete, cx);
                    }))
                    .child(crate::locale::tr("Play preview", "试听音效")),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn settings_sounds_toggle_is_global_and_preview_is_only_available_when_enabled(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, vcx) = cx.add_window_view(|_, cx| {
            sounds::set_enabled(false, cx);
            let mut workspace = Workspace::for_test(learning::Coach::new(), cx);
            workspace.sidebar_view = SidebarView::Settings;
            workspace
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("settings-sounds-preview").is_none());
        let button = vcx.debug_bounds("settings-sounds").unwrap();
        vcx.simulate_click(button.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(workspace.read_with(vcx, |_, cx| sounds::enabled(cx)));
        let button = vcx.debug_bounds("settings-sounds-preview").unwrap();
        vcx.simulate_click(button.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        workspace.update(vcx, |_, cx| {
            assert_eq!(sounds::drain_requested(cx), [Cue::Complete, Cue::Complete]);
        });
        let button = vcx.debug_bounds("settings-sounds").unwrap();
        vcx.simulate_click(button.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(!workspace.read_with(vcx, |_, cx| sounds::enabled(cx)));
        assert!(vcx.debug_bounds("settings-sounds-preview").is_none());
    }

    #[gpui::test]
    fn settings_display_pills_and_archive_search_filter_work(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let original = crate::config::display();
        let (workspace, vcx) = cx.add_window_view(|_, cx| {
            let mut workspace = Workspace::for_test(learning::Coach::new(), cx);
            workspace.sidebar_view = SidebarView::Settings;
            workspace
        });
        vcx.run_until_parked();
        let click = |vcx: &mut gpui::VisualTestContext, selector: &'static str| {
            let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
            vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
        };
        click(vcx, "settings-thinking-display-collapsed");
        assert_eq!(crate::config::display().value("thinking_display").as_deref(), Some("collapsed"));
        click(vcx, "settings-steps-display-expanded");
        assert_eq!(crate::config::display().value("steps_display").as_deref(), Some("expanded"));
        click(vcx, "settings-diff-display-pinned");
        assert_eq!(crate::config::display().value("diff_display").as_deref(), Some("pinned"));
        let _ = crate::config::set_display("thinking_display", &original.value("thinking_display").unwrap());
        let _ = crate::config::set_display("steps_display", &original.value("steps_display").unwrap());
        let _ = crate::config::set_display("diff_display", &original.value("diff_display").unwrap());

        workspace.update(vcx, |w, cx| {
            w.sidebar_view = SidebarView::Sessions;
            let mut alpha = crate::workspace::tests::session_info("arch-alpha", Some("Alpha refactor"));
            alpha.archived = true;
            let mut beta = crate::workspace::tests::session_info("arch-beta", Some("Beta deployment"));
            beta.archived = true;
            w.load_sidebar_archive(Ok(vec![alpha, beta]));
            cx.notify();
        });
        vcx.run_until_parked();
        click(vcx, "sidebar-archive-toggle");
        assert!(vcx.debug_bounds("sidebar-archive-search").is_some());
        click(vcx, "sidebar-archive-search");
        vcx.simulate_input("beta");
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("sidebar-archive-list").is_some());
    }
}
