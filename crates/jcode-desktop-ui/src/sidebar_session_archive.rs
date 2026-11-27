use super::*;

#[derive(Default)]
pub(super) struct State {
    pending: HashSet<String>,
    archived: Vec<jcode_sdk::SessionInfo>,
    close: Vec<String>,
    show: bool,
    loading: bool,
    reload_after_list: bool,
    error: Option<String>,
}

impl State {
    pub(super) fn excludes(&self, id: &str) -> bool {
        self.archived.iter().any(|session| session.session_id == id)
    }
}

fn button(id: &'static str, text: &'static str) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div().id(id).debug_selector(move || id.into())
        .px_2().py_1().rounded_md().cursor_pointer().text_size(px(11.0))
        .hover(|el| el.bg(theme.TOOL_BG)).child(text)
}

impl Workspace {
    pub(super) fn sidebar_archive_blocker(&self, id: &str, cx: &App) -> Option<&'static str> {
        if self.session_archive.pending.contains(id) {
            return Some(tr("Archiving conversation…", "正在归档会话…"));
        }
        if Panel::is_pending_session_id(id)
            || !self.sessions.iter().any(|session| session.session_id == id)
        {
            return Some(tr("Conversation is not ready yet", "会话尚未就绪"));
        }
        self.slots.iter().any(|slot| {
            let panel = slot.panel.read(cx);
            panel.session_id == id && panel.activity_active()
        }).then(|| tr("Wait for the current response to finish", "请等待当前回复结束"))
    }

    fn can_archive_sidebar_session(&self, id: &str, cx: &App) -> bool {
        self.sidebar_archive_blocker(id, cx).is_none()
    }

    pub(super) fn confirm_sidebar_archive(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_archive_sidebar_session(&id, cx) {
            return;
        }
        let title = self.sessions.iter().find(|session| session.session_id == id)
            .and_then(|session| session.title.as_deref()).unwrap_or(&id);
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            tr("Delete conversation?", "删除会话？"),
            Some(&format!("{}\n\n{}", title, tr(
                "This removes the conversation from the sidebar and keeps it in Archived conversations for recovery.",
                "会话将从侧栏移除，记录可在「已归档会话」中恢复。",
            ))),
            &[tr("Delete conversation", "删除会话"), tr("Cancel", "取消")],
            cx,
        );
        self.collections.menu = None;
        cx.notify();
        cx.spawn(async move |workspace, cx| {
            if prompt.await == Ok(0) {
                let _ = workspace.update(cx, |this, cx| {
                    if this.can_archive_sidebar_session(&id, cx) {
                        this.set_sidebar_session_archived(id, true, cx);
                    }
                });
            }
        }).detach();
    }

    fn set_sidebar_session_archived(&mut self, id: String, archived: bool, cx: &mut Context<Self>) {
        if self.session_archive.pending.insert(id.clone()) {
            self.session_archive.error = None;
            self.bridge.send(Command::SetSessionArchived { session_id: id, archived });
            cx.notify();
        }
    }

    pub(super) fn settle_sidebar_archive(&mut self, id: String, archived: bool, error: Option<String>) {
        self.session_archive.pending.remove(&id);
        if let Some(error) = error {
            let label = if archived { tr("Could not archive conversation.", "无法归档会话。") } else { tr("Could not restore conversation.", "无法恢复会话。") };
            self.session_archive.error = Some(format!("{label} {error}"));
            return;
        }
        self.session_archive.error = None;
        self.session_archive.reload_after_list |= self.session_archive.loading;
        if archived {
            if let Some(index) = self.sessions.iter().position(|session| session.session_id == id) {
                let mut session = self.sessions.remove(index);
                session.archived = true;
                self.session_archive.archived.insert(0, session);
            }
            self.session_archive.close.push(id);
        } else {
            self.session_archive.archived.retain(|session| session.session_id != id);
        }
        self.bridge.send(Command::RefreshSessions);
    }

    pub(super) fn load_sidebar_archive(&mut self, result: Result<Vec<jcode_sdk::SessionInfo>, String>) {
        if std::mem::take(&mut self.session_archive.reload_after_list) {
            if self.session_archive.show {
                self.bridge.send(Command::ListArchivedSessions);
            } else {
                self.session_archive.loading = false;
            }
            return;
        }
        self.session_archive.loading = false;
        match result {
            Ok(sessions) => {
                self.session_archive.archived.retain(|s| harness::remote_host(&s.session_id).is_some());
                self.session_archive.archived.extend(sessions);
            }
            Err(error) => self.session_archive.error = Some(format!("{} {error}", tr("Could not load archived conversations.", "无法加载已归档会话。"))),
        }
    }

    pub(super) fn close_archived_sidebar_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = std::mem::take(&mut self.session_archive.close);
        if ids.iter().any(|id| self.slots.iter().any(|slot| !slot.closing && slot.panel.read(cx).session_id == *id)) {
            self.close_sidebar_sessions(ids, window, cx);
        }
    }

    pub(super) fn render_sidebar_archive_toggle(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let label = if self.session_archive.show {
            tr("Back to conversations", "返回会话")
        } else {
            tr("Archived conversations", "已归档会话")
        };
        button("sidebar-archive-toggle", label)
            .flex_none().mx_2().text_color(Theme::global().TEXT_DIM)
            .on_click(cx.listener(|this, _, _, cx| {
                let state = &mut this.session_archive;
                state.show = !state.show;
                if state.show && !state.loading {
                    state.loading = true;
                    state.error = None;
                    this.bridge.send(Command::ListArchivedSessions);
                }
                cx.notify();
            })).into_any_element()
    }

    pub(super) fn render_sidebar_archives(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let state = &self.session_archive;
        if !state.show {
            return None;
        }
        let theme = Theme::global();
        let mut list = div().id("sidebar-archive-list").flex_1().min_h_0().overflow_y_scroll().p_2();
        if state.loading {
            list = list.child(div().p_2().child(tr("Loading archived conversations…", "正在加载已归档会话…")));
        } else if state.archived.is_empty() && state.error.is_none() {
            list = list.child(div().p_2().child(tr("No archived conversations", "暂无已归档会话")));
        }
        for session in &state.archived {
            let id = session.session_id.clone();
            let pending = state.pending.contains(&id);
            list = list.child(div().id(gpui::SharedString::from(format!("archived-session-{id}")))
                .flex().items_center().gap_1().py_1().border_b_1().border_color(theme.PANEL_BORDER)
                .child(div().flex_1().min_w_0().truncate().text_size(px(11.0))
                    .child(session.title.clone().unwrap_or_else(|| id.clone())))
                .child(button("restore-archived-session", if pending { tr("Restoring…", "正在恢复…") } else { tr("Restore", "恢复") })
                    .flex_none().text_color(theme.ACCENT)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_sidebar_session_archived(id.clone(), false, cx);
                    }))));
        }
        Some(list.into_any_element())
    }

    pub(super) fn render_sidebar_archive_error(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let error = self.session_archive.error.as_ref()?;
        Some(div().flex_none().mx_2().p_1().flex().items_start().gap_1().bg(Theme::global().ERROR_BG)
            .child(div().flex_1().min_w_0().text_size(px(11.0)).text_color(Theme::global().ERROR).child(error.clone()))
            .child(button("sidebar-archive-error-close", "×").flex_none().on_click(cx.listener(|this, _, _, cx| {
                this.session_archive.error = None;
                cx.notify();
            }))).into_any_element())
    }
}
