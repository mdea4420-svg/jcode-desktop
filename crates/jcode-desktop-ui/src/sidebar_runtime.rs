use super::*;
use crate::custom_providers::{self, RuntimeReloadStatus};
use crate::locale::tr;

#[derive(Default)]
pub(super) struct State {
    focus: Option<gpui::FocusHandle>,
    reloading: bool,
    notice: Option<&'static str>,
}

impl Workspace {
    fn runtime_host(&self, cx: &App) -> Option<String> {
        let slot = self.slots.get(self.active).filter(|slot| !slot.closing)?;
        harness::remote_host(&slot.panel.read(cx).session_id)
    }

    fn runtime_connection_label(&self) -> &'static str {
        if self.connected {
            tr("Connected", "已连接")
        } else {
            tr("Reconnecting…", "重连中…")
        }
    }

    fn live_runtime_sessions<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = &'a str> {
        self.slots.iter().filter(|slot| !slot.closing).filter_map(move |slot| {
            let panel = slot.panel.read(cx);
            let id = panel.session_id.as_str();
            (id != "terminal"
                && !panel.is_pending_session()
                && !panel.is_side_document()
                && !panel.is_change_review()
                && !panel.is_accounts_panel()
                && !panel.is_changelog()
                && !panel.is_machines()
                && id != Panel::DEFAULT_DIRECTORY_SESSION_ID
                && panel.preview_state.is_none())
                .then_some(id)
        })
    }

    fn refresh_sidebar_runtime(&mut self, cx: &mut Context<Self>) {
        self.bridge.send(Command::RefreshSessions);
        let mut sent = HashSet::new();
        for session_id in self.live_runtime_sessions(cx) {
            if sent.insert(session_id) {
                self.bridge.send(Command::RefreshRuntime { session_id: session_id.into() });
            }
        }
        self.sidebar_runtime.notice = Some(tr("Refresh requested.", "已请求刷新。"));
        cx.notify();
    }

    fn reload_sidebar_runtime(&mut self, cx: &mut Context<Self>) {
        if self.runtime_host(cx).is_some() || self.sidebar_runtime.reloading {
            return;
        }
        if cfg!(test) || harness::screenshot_mode() {
            return;
        }
        self.sidebar_runtime.reloading = true;
        self.sidebar_runtime.notice = None;
        let task = cx.background_executor().spawn(async {
            custom_providers::reload_runtime_server_gracefully()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.sidebar_runtime.reloading = false;
                workspace.sidebar_runtime.notice = Some(match result {
                    RuntimeReloadStatus::Reloaded => tr("Daemon reloaded or already current.", "本机服务已重载或已是最新。"),
                    RuntimeReloadStatus::NotRunning => tr("The local daemon is not running.", "本机服务未运行。"),
                    RuntimeReloadStatus::Deferred => tr("Reload did not complete. Active work was not force-stopped.", "暂未完成重载，未强制中断后台任务。"),
                });
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn copy_runtime_diagnostics(&mut self, cx: &mut Context<Self>) {
        // Use fixed fields only. Runtime messages and host addresses can contain private data.
        let target = if self.runtime_host(cx).is_some() { "remote" } else { "local" };
        let diagnostics = format!(
            "Jcode Desktop {}\nRuntime target: {target}\nConnected: {}\nOpen session panels: {}\nBackground sessions: {}",
            crate::build_info::VERSION,
            self.connected,
            self.live_runtime_sessions(cx).count(),
            self.daemon_running.len(),
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(diagnostics));
        self.sidebar_runtime.notice = Some(tr("Diagnostics copied.", "已复制诊断信息。"));
        cx.notify();
    }

    pub(super) fn close_sidebar_runtime(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_runtime.focus.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_sidebar_runtime(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_runtime.focus.is_some() {
            self.close_sidebar_runtime(cx);
            self.focus_active(window, cx);
        } else {
            self.close_account_menu(cx);
            self.account_menu.open = false;
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            self.sidebar_runtime.focus = Some(focus);
            cx.notify();
        }
    }

    pub(super) fn render_sidebar_runtime(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let remote = self.runtime_host(cx);
        let target = match remote.as_deref() {
            Some(host) => format!("{} · {host}", tr("Remote", "远端")),
            None => tr("Local runtime", "本机运行时").into(),
        };
        let running = self.daemon_running.len();
        let mut status = self.runtime_connection_label().to_owned();
        if self.connected && running > 0 {
            status.push_str(&format!(" · {running} {}", tr("running", "个运行中")));
        }
        div()
            .id("sidebar-runtime")
            .flex()
            .flex_col()
            .gap_1()
            .when_some(self.sidebar_runtime.focus.as_ref(), |area, focus| {
                area.track_focus(focus)
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_sidebar_runtime(cx)))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.close_sidebar_runtime(cx);
                            this.focus_active(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(self.render_runtime_popover(&target, &status, remote.is_some(), cx))
            })
            .child(
                div()
                    .id("sidebar-runtime-status")
                    .debug_selector(|| "sidebar-runtime-status".into())
                    .h(px(25.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_full()
                    .bg(theme.INLINE_CODE_BG)
                    .hover(|pill| pill.bg(theme.HEADER_BG))
                    .text_size(px(10.5))
                    .cursor_pointer()
                    .child(div().flex_1().min_w_0().truncate().text_color(theme.TEXT_DIM).child(target))
                    .child(div().min_w_0().truncate().text_color(if self.connected { theme.OK } else { theme.WARN }).child(status))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar_runtime(window, cx))),
            )
            .into_any_element()
    }

    fn render_runtime_popover(&self, target: &str, status: &str, remote: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let can_reload = !remote && !self.sidebar_runtime.reloading;
        let open = self.live_runtime_sessions(cx).count();
        let running = self.daemon_running.len();
        let reload_label = if self.sidebar_runtime.reloading {
            tr("Reloading…", "正在重载…")
        } else {
            tr("Safe reload daemon", "安全重载本机服务")
        };
        div()
            .id("sidebar-runtime-popover")
            .debug_selector(|| "sidebar-runtime-popover".into())
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_xl()
            .bg(theme.HEADER_BG)
            .text_color(theme.TEXT)
            .text_size(px(11.))
            .child(div().truncate().child(target.to_owned()))
            .child(div().truncate().child(status.to_owned()))
            .when(!self.status.is_empty(), |popover| {
                popover.child(div().truncate().text_color(theme.TEXT_DIM).child(self.status.clone()))
            })
            .child(format!("{} · {open}", tr("Open session panels", "已打开会话面板")))
            .child(format!("{} · {running}", tr("Background sessions", "后台运行会话")))
            .child(runtime_button("runtime-reconnect", tr("Reconnect & refresh", "重新连接并刷新"), true)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_sidebar_runtime(cx))))
            .child(runtime_button("runtime-safe-reload", reload_label, can_reload)
                .when(can_reload, |button| button.on_click(cx.listener(|this, _, _, cx| this.reload_sidebar_runtime(cx)))))
            .when(remote, |popover| {
                popover.child(
                    div()
                        .debug_selector(|| "runtime-remote-reload-note".into())
                        .text_color(theme.TEXT_DIM)
                        .child(tr(
                            "The remote daemon runs on its own host.",
                            "远端服务运行在远端主机上。",
                        )),
                )
            })
            .child(runtime_button("runtime-copy-diagnostics", tr("Copy diagnostics", "复制诊断信息"), true)
                .on_click(cx.listener(|this, _, _, cx| this.copy_runtime_diagnostics(cx))))
            .when_some(self.sidebar_runtime.notice, |popover, notice| {
                popover.child(div().text_color(theme.TEXT_DIM).child(notice))
            })
            .into_any_element()
    }
}

fn runtime_button(id: &'static str, label: &'static str, enabled: bool) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .rounded_full()
        .px_2()
        .py_1()
        .bg(theme.INLINE_CODE_BG)
        .when(enabled, |button| button.cursor_pointer().hover(|button| button.bg(theme.ACCENT_DIM)))
        .when(!enabled, |button| button.text_color(theme.TEXT_FAINT))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn sidebar_runtime_refreshes_open_sessions_and_blocks_remote_reload(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let (bridge, commands) = harness::spawn_recording();
        let (workspace, vcx) = cx.add_window_view(|_, cx| {
            let mut workspace = Workspace::for_test(crate::learning::Coach::new(), cx);
            workspace.set_test_bridge(bridge);
            workspace.push_test_panel("ssh://fixture-host/remote-session", cx);
            workspace.push_test_panel("local-session", cx);
            workspace.active = 0;
            workspace.status = "sample status with account@example.com".into();
            workspace
        });
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(1300.), px(820.)));
        vcx.run_until_parked();
        while commands.try_recv().is_ok() {}
        workspace.read_with(vcx, |workspace, cx| {
            assert_eq!(workspace.runtime_host(cx).as_deref(), Some("fixture-host"));
            assert_eq!(workspace.runtime_connection_label(), tr("Connected", "已连接"));
        });
        let status = vcx.debug_bounds("sidebar-runtime-status").unwrap();
        let account = vcx.debug_bounds("sidebar-account").unwrap();
        assert!(status.bottom() <= account.top());
        vcx.simulate_click(status.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("sidebar-runtime-popover").is_some());
        assert!(vcx.debug_bounds("runtime-remote-reload-note").is_some());
        let reload = vcx.debug_bounds("runtime-safe-reload").unwrap();
        vcx.simulate_click(reload.center(), gpui::Modifiers::default());
        workspace.read_with(vcx, |workspace, _| assert!(!workspace.sidebar_runtime.reloading));
        assert!(commands.try_recv().is_err());
        let reconnect = vcx.debug_bounds("runtime-reconnect").unwrap();
        vcx.simulate_click(reconnect.center(), gpui::Modifiers::default());
        assert!(matches!(commands.try_recv(), Ok(Command::RefreshSessions)));
        for expected in ["ssh://fixture-host/remote-session", "local-session"] {
            assert!(matches!(commands.try_recv(), Ok(Command::RefreshRuntime { session_id }) if session_id == expected));
        }
        assert!(commands.try_recv().is_err());
        vcx.run_until_parked();
        let copy = vcx.debug_bounds("runtime-copy-diagnostics").unwrap();
        vcx.simulate_click(copy.center(), gpui::Modifiers::default());
        let diagnostics = vcx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap();
        assert!(diagnostics.contains("Runtime target: remote"));
        assert!(!diagnostics.contains("account@example.com"));
        assert!(!diagnostics.contains("fixture-host"));
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("sidebar-runtime-popover").is_none());
    }
}
