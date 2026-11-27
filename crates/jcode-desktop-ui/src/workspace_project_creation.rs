use super::*;
use gpui::{FontWeight, PathPromptOptions};

pub(super) const PROJECT_NAME_LIMIT: usize = 80;

impl Workspace {
    pub(super) fn project_key_for_directory(&mut self, directory: &str) -> String {
        let directory = directory.trim();
        self.sidebar_projects.root(directory)
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_else(|| directory.to_owned())
    }

    fn active_project_key(&mut self, cx: &App) -> Option<String> {
        let panel = self.slots.get(self.active)?.panel.read(cx);
        if harness::remote_host(&panel.session_id).is_some()
            || pending::remote_draft_host(&panel.session_id).is_some()
        {
            return None;
        }
        let directory = panel.working_dir.as_deref()?;
        Some(self.project_key_for_directory(directory))
    }

    pub(super) fn refresh_project_labels(&mut self, cx: &mut Context<Self>) {
        self.project_menu.load();
        let home = crate::platform_dirs::home_string();
        for index in 0..self.slots.len() {
            if self.slots[index].closing {
                continue;
            }
            let panel = self.slots[index].panel.clone();
            let chat = panel.read(cx);
            let directory = chat.working_dir.as_deref().filter(|_| chat.is_untouched_empty_chat());
            let label = directory.and_then(|directory| {
                let key = self.project_key_for_directory(directory);
                let prefs = self.project_menu.get(&key);
                if sidebar_projects::is_noise_project_key(&key, home.as_deref(), prefs)
                    || prefs.is_some_and(|p| p.removed)
                {
                    return None;
                }
                Some(prefs.and_then(|p| p.name.clone()).unwrap_or_else(|| sidebar_projects::path_label(&key)))
            });
            if chat.project_folder_label != label {
                panel.update(cx, |panel, cx| {
                    panel.project_folder_label = label;
                    cx.notify();
                });
            }
        }
    }

    fn known_project_folders(&mut self, cx: &App) -> Vec<sidebar_projects::Project> {
        self.project_menu.load();
        let mut projects = self.registered_sidebar_projects();
        let mut seen = projects.iter().map(|p| p.key.clone()).collect::<HashSet<_>>();
        let home = crate::platform_dirs::home_string();
        let live = self.slots.iter().filter(|slot| !slot.closing).filter_map(|slot| {
            let panel = slot.panel.read(cx);
            panel.supports_voice().then(|| pending::draft_session_info(
                panel.session_id.clone(), None, panel.working_dir.clone(),
            ))
        }).collect::<Vec<_>>();
        for session in self.sessions.iter().chain(&live) {
            if harness::remote_host(&session.session_id).is_some()
                || pending::remote_draft_host(&session.session_id).is_some()
                || self.session_archive.excludes(&session.session_id)
            {
                continue;
            }
            let (mut project, _) = self.sidebar_projects.project(session);
            let prefs = self.project_menu.get(&project.key);
            if !Path::new(&project.key).is_absolute()
                || sidebar_projects::is_noise_project_key(&project.key, home.as_deref(), prefs)
                || !seen.insert(project.key.clone())
            {
                continue;
            }
            if let Some(name) = prefs.and_then(|p| p.name.as_ref()) {
                project.label.clone_from(name);
            }
            projects.push(project);
        }
        projects.retain(|p| !self.project_menu.get(&p.key).is_some_and(|p| p.removed || p.archived));
        projects.sort_by_cached_key(|p| (
            !self.project_menu.get(&p.key).is_some_and(|prefs| prefs.pinned),
            p.label.to_lowercase(),
            p.key.clone(),
        ));
        projects
    }

    pub(super) fn toggle_project_folder_menu(
        &mut self,
        action: &crate::ui_actions::ToggleProjectFolderMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(source) = action.source {
            let Some(index) = self.slots.iter().position(|slot| {
                !slot.closing && slot.panel.entity_id() == source
            }) else {
                return;
            };
            self.set_active(index, cx);
        }
        self.project_menu.close();
        self.collections.menu = None;
        self.project_folder_menu = self.project_folder_menu.is_none().then_some(action.position);
        cx.notify();
    }

    pub(super) fn dismiss_project_folder_menu_on_escape(
        &mut self,
        event: &gpui::KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.project_folder_menu.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(super) fn render_project_folder_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        use super::sidebar_project_menu::{popup_row, popup_separator, popup_surface};

        let anchor = self.project_folder_menu? + gpui::point(px(6.0), px(8.0));
        let viewport = window.viewport_size();
        let known = self.known_project_folders(cx);
        let current = self.active_project_key(cx);
        let menu = popup_surface("project-folder-menu", viewport.height)
            .w(px(232.0))
            .max_w((viewport.width - px(16.0)).max(px(1.0)))
            .children(known.iter().enumerate().map(|(index, project)| {
                let id = format!("project-folder-known-{index}");
                let key = project.key.clone();
                let selected = current.as_deref() == Some(key.as_str());
                popup_row(gpui::SharedString::from(id.clone()).into(), id, false)
                    .child(sidebar_worktrees::icon(sidebar_worktrees::FOLDER_ICON, 12.0))
                    .child(div().flex_1().min_w_0().truncate().child(project.label.clone()))
                    .when(selected, |row| row.child(div()
                        .debug_selector(move || format!("project-folder-known-selected-{index}"))
                        .child("✓")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.project_folder_menu = None;
                        this.activate_or_open_project_directory(key.clone(), cx);
                    }))
            }))
            .when(!known.is_empty(), |menu| menu.child(popup_separator()))
            .children([
                ("project-folder-open", tr("Open existing folder…", "打开已有文件夹…"), false),
                ("project-folder-create", tr("Create project folder…", "新建项目文件夹…"), true),
            ].into_iter().map(|(id, label, create)| {
                popup_row(id.into(), id.into(), false)
                    .child(div().min_w_0().truncate().child(label))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if create {
                            this.show_project_folder_picker(true, window, cx);
                        } else {
                            this.choose_existing_project_folder(window, cx);
                        }
                    }))
            }));
        Some(div()
            .absolute()
            .inset_0()
            .child(div().absolute().inset_0().occlude().on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.project_folder_menu = None;
                    cx.notify();
                }),
            ))
            .child(gpui::deferred(gpui::anchored().position(anchor).child(menu)).with_priority(10))
            .into_any_element())
    }

    pub(super) fn choose_existing_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_folder_menu = None;
        self.close_folder_picker(cx);
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Select project folder", "选择项目文件夹").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = selection.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    let directory = path.to_string_lossy().into_owned();
                    this.register_project(&directory);
                    this.activate_or_open_project_directory(directory, cx);
                });
            }
        }).detach();
    }

    pub(super) fn render_sidebar_projects_section(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let button = sidebar_worktrees::header_action(
            "sidebar-project-folder".into(),
            tr("Open or create a project folder", "打开或创建项目文件夹"),
            crate::ui_actions::project_folder_plus_icon().into_any_element(),
            true,
            "sidebar-projects-section",
        )
        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
            this.toggle_project_folder_menu(
                &crate::ui_actions::ToggleProjectFolderMenu { source: None, position: event.position },
                window,
                cx,
            );
        }));
        div()
            .flex_none()
            .child(sidebar_section_heading("sidebar-projects-section", tr("Projects", "项目"), button))
            .when(self.project_menu.has_removed(), |el| el.child(self.render_removed_projects_toggle(cx)))
            .into_any_element()
    }

    pub(super) fn render_sidebar_collections_section(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let button = sidebar_worktrees::header_action(
            "sidebar-new-collection".into(),
            tr("Create a collection to group sessions", "创建集合，将会话归组"),
            div().child("+").into_any_element(),
            true,
            "sidebar-collections-section",
        )
        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
            this.begin_new_collection(None, window, cx);
        }));
        sidebar_section_heading("sidebar-collections-section", tr("Collections", "集合"), button)
            .into_any_element()
    }

    fn choose_project_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.folder_search.as_ref().map(Entity::entity_id) else {
            return;
        };
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose a project location", "选择项目位置").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = selection.await;
            let _ = this.update(cx, |this, cx| {
                if !this.folder_picker_creates_project
                    || this.folder_search.as_ref().map(Entity::entity_id) != Some(input)
                {
                    return;
                }
                match selected {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.browse_to(path, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        this.folder_picker_error = Some(tr(
                            "Could not open the system folder picker.",
                            "无法打开系统文件夹选择器。",
                        ).into());
                        cx.notify();
                    }
                }
            });
        }).detach();
    }

    pub(super) fn render_project_creation(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let Some(input) = self.folder_search.clone() else {
            return div().into_any_element();
        };
        let draft = input.read(cx).content.to_string();
        let name = draft.trim();
        let count = draft.chars().count();
        let parent = self.folder_picker_dir.as_ref();
        let enabled = count > 0 && count <= PROJECT_NAME_LIMIT && parent.is_some();
        let location = parent.map(|path| path.display().to_string());
        let target = parent.filter(|_| valid_project_name(name)).map(|path| path.join(name));
        let cancel = || cx.listener(|this, _, _window, cx| {
            cx.stop_propagation();
            this.close_folder_picker(cx);
        });

        div()
            .id("project-creation-overlay")
            .debug_selector(|| "folder-picker-overlay".into())
            .absolute().inset_0().flex().items_center().justify_center()
            .bg(gpui::rgba(0x00000070))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.focus_pending = false;
                if let Some(search) = &this.folder_search {
                    window.focus(&search.read(cx).focus_handle.clone(), cx);
                }
            }))
            .child(div()
                .w(px(560.)).max_w(relative(0.92)).max_h(relative(0.9))
                .id("project-creation-form")
                .flex().flex_col().gap_4().p_5().overflow_y_scroll()
                .rounded_xl().border_1().border_color(theme.PANEL_BORDER).bg(theme.PANEL_BG)
                .font_family(theme.FONT_UI).text_color(theme.TEXT)
                .child(div().flex().items_center().justify_between().gap_3()
                    .child(div().text_size(px(19.)).font_weight(FontWeight::SEMIBOLD)
                        .child(tr("Create project", "创建项目")))
                    .child(div().id("project-creation-close").px_2().py_1().rounded_md()
                        .cursor_pointer().text_color(theme.TEXT_DIM).hover(|el| el.bg(theme.HEADER_BG))
                        .on_click(cancel()).child("×")))
                .child(div().flex().flex_col().gap_2()
                    .child(div().flex().justify_between()
                        .child(tr("Project name", "项目名称"))
                        .child(div().text_color(if count > PROJECT_NAME_LIMIT { theme.ERROR } else { theme.TEXT_DIM })
                            .child(format!("{count}/{PROJECT_NAME_LIMIT}"))))
                    .child(div().px_3().py_2().rounded_lg().bg(theme.INPUT_BG)
                        .border_1().border_color(theme.INPUT_BORDER).child(input)))
                .child(div().flex().flex_col().gap_2()
                    .child(div().flex().items_center().justify_between()
                        .child(tr("Workspace location", "工作区位置"))
                        .child(div().text_sm().text_color(theme.TEXT_DIM).child(tr("This computer", "此电脑"))))
                    .child(div().id("project-creation-location")
                        .debug_selector(|| "project-creation-location".into())
                        .flex().items_center().gap_3().px_3().py_3().rounded_lg()
                        .border_1().border_color(theme.PANEL_BORDER).bg(theme.HEADER_BG)
                        .cursor_pointer().hover(|el| el.border_color(theme.ACCENT))
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.choose_project_parent(window, cx);
                        }))
                        .child(div().flex_1().min_w_0().truncate().text_color(theme.TEXT_DIM)
                            .child(location.unwrap_or_else(|| tr("No location selected", "尚未选择位置").into())))
                        .child(div().flex_none().text_color(theme.ACCENT)
                            .child(tr("Choose folder…", "选择文件夹…"))))
                    .when_some(target, |el, path| el.child(div().text_sm().text_color(theme.TEXT_DIM)
                        .child(if is_zh() { format!("将创建于 {}", path.display()) }
                            else { format!("Will create at {}", path.display()) }))))
                .when_some(self.folder_picker_error.clone(), |el, error| {
                    el.child(div().debug_selector(|| "folder-picker-error".into())
                        .text_sm().text_color(theme.ERROR).child(error))
                })
                .child(div().flex().justify_end().gap_2()
                    .child(div().id("project-creation-cancel").debug_selector(|| "folder-picker-cancel".into())
                        .px_3().py_2().rounded_md().text_color(theme.TEXT_DIM).cursor_pointer()
                        .hover(|el| el.bg(theme.HEADER_BG)).on_click(cancel()).child(tr("Cancel", "取消")))
                    .child(div().id("project-creation-submit").debug_selector(|| "folder-picker-open".into())
                        .px_4().py_2().rounded_md()
                        .bg(if enabled { theme.ACCENT } else { theme.HEADER_BG })
                        .text_color(if enabled { theme.BG } else { theme.TEXT_DIM })
                        .when(enabled, |el| el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.create_project_folder(&draft, cx);
                        })))
                        .child(tr("Create project", "创建项目")))))
            .into_any_element()
    }
}

pub(super) fn valid_project_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(part)) if part == std::ffi::OsStr::new(name))
        && components.next().is_none()
        && name.chars().count() <= PROJECT_NAME_LIMIT
        && !name.contains(|c: char| c.is_control() || matches!(c, '/' | '\\' | ':'))
        && !(cfg!(windows) && (name.ends_with('.') || name.contains(['<', '>', '"', '|', '?', '*'])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn chinese_project_shortcut_and_quick_switch_preserve_the_empty_slot(cx: &mut gpui::TestAppContext) {
        crate::locale::with_zh(|| {
            let root = tempfile::tempdir().unwrap();
            let first = root.path().join("alpha");
            let second = root.path().join("beta");
            std::fs::create_dir(&first).unwrap();
            std::fs::create_dir(&second).unwrap();
            let second_key = second.to_string_lossy().into_owned();
            let (bridge, commands) = harness::spawn_recording();
            cx.update(crate::bind_workspace_keys);
            let (workspace, vcx) = cx.add_window_view(|_, cx| {
                let mut workspace = Workspace::for_test(learning::Coach::new(), cx);
                workspace.bridge = bridge;
                workspace.register_project(&second_key);
                workspace.open_local_draft(None, cx);
                workspace
            });
            let Command::CreateSession { request_id: Some(old_request), .. } = commands.try_recv().unwrap() else {
                panic!("expected initial draft");
            };
            let (row, width) = workspace.update(vcx, |workspace, cx| {
                let slot = &workspace.slots[workspace.active];
                slot.panel.read(cx).input.clone().update(cx, |input, cx| input.set_content("尚未发送".into(), cx));
                (slot.row, slot.width_fraction)
            });
            vcx.run_until_parked();
            vcx.update(|window, cx| window.dispatch_action(Box::new(OpenFolder), cx));
            vcx.simulate_path_prompt_response(|options| {
                assert!(options.directories && !options.files && !options.multiple);
                assert_eq!(options.prompt.as_deref(), Some("选择项目文件夹"));
                Some(vec![first.clone()])
            });
            vcx.run_until_parked();
            let Command::CreateSession { working_dir: Some(directory), request_id: Some(first_request) } = commands.try_recv().unwrap() else {
                panic!("expected project draft");
            };
            assert_eq!(Path::new(&directory), first.as_path());
            workspace.update(vcx, |workspace, cx| {
                assert_eq!(workspace.slots[workspace.active].panel.read(cx).project_folder_label.as_deref(), Some("alpha"));
                workspace.apply(Update::SessionCreated {
                    session: pending::draft_session_info("old-session".into(), None, None),
                    request_id: Some(old_request),
                }, cx);
            });
            assert!(matches!(commands.try_recv().unwrap(), Command::Unwatch { session_id } if session_id == "old-session"));
            workspace.update(vcx, |workspace, cx| {
                workspace.apply(Update::SessionCreated {
                    session: pending::draft_session_info("connected-session".into(), None, Some(directory)),
                    request_id: Some(first_request),
                }, cx);
                workspace.apply(Update::SessionConnected { session_id: "connected-session".into() }, cx);
                let panel = workspace.slots[workspace.active].panel.read(cx);
                assert_eq!(panel.status, "connected");
                assert_eq!(panel.input.read(cx).content.as_ref(), "尚未发送");
                panel.input.clone().update(cx, |input, cx| input.set_content("连接后编辑的草稿".into(), cx));
                cx.notify();
            });
            vcx.run_until_parked();

            let button = vcx.debug_bounds("composer-project-folder").unwrap();
            vcx.simulate_click(button.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
            assert!(vcx.debug_bounds("project-folder-known-selected-0").is_some());
            let choice = vcx.debug_bounds("project-folder-known-1").unwrap();
            vcx.simulate_click(choice.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
            assert!(matches!(commands.try_recv().unwrap(), Command::Unwatch { session_id } if session_id == "connected-session"));
            assert!(matches!(commands.try_recv().unwrap(), Command::CreateSession { working_dir: Some(directory), .. } if directory == second_key));
            workspace.read_with(vcx, |workspace, cx| {
                assert_eq!(workspace.slots.len(), 1);
                let slot = &workspace.slots[workspace.active];
                assert_eq!((slot.row, slot.width_fraction), (row, width));
                let panel = slot.panel.read(cx);
                assert_eq!(panel.input.read(cx).content.as_ref(), "连接后编辑的草稿");
                assert_eq!(panel.project_folder_label.as_deref(), Some("beta"));
                assert!(workspace.sessions.is_empty());
            });
        });
    }

    #[gpui::test]
    fn project_badge_on_inactive_split_switches_only_its_own_slot(cx: &mut gpui::TestAppContext) {
        crate::locale::with_zh(|| {
            let root = tempfile::tempdir().unwrap();
            let alpha = root.path().join("alpha");
            let beta = root.path().join("beta");
            std::fs::create_dir(&alpha).unwrap();
            std::fs::create_dir(&beta).unwrap();
            let alpha = alpha.to_string_lossy().into_owned();
            let beta = beta.to_string_lossy().into_owned();
            let (bridge, commands) = harness::spawn_recording();
            let (workspace, vcx) = cx.add_window_view(|_, cx| {
                let mut workspace = Workspace::for_test(learning::Coach::new(), cx);
                workspace.bridge = bridge;
                workspace.layout_mode = crate::config::LayoutMode::Normal;
                workspace.show_sidebar = false;
                workspace.open_local_draft(Some(alpha.clone()), cx);
                workspace.open_local_draft(Some(beta.clone()), cx);
                workspace.set_active(0, cx);
                workspace
            });
            let window = vcx.update(|window, _| window.window_handle());
            vcx.simulate_window_resize(window, gpui::size(px(1600.0), px(800.0)));
            let (left, right) = workspace.update(vcx, |workspace, cx| {
                let left = workspace.slots[0].panel.clone();
                let right = workspace.slots[1].panel.clone();
                for (panel, draft) in [(&left, "left draft"), (&right, "right draft")] {
                    panel.read(cx).input.clone().update(cx, |input, cx| input.set_content(draft.into(), cx));
                }
                (left, right)
            });
            vcx.run_until_parked();
            assert_eq!(commands.try_iter().count(), 2);
            workspace.read_with(vcx, |workspace, cx| {
                assert_eq!(workspace.active, 0);
                assert_eq!(left.read(cx).project_folder_label.as_deref(), Some("alpha"));
                assert_eq!(right.read(cx).project_folder_label.as_deref(), Some("beta"));
            });
            let selector = format!("composer-project-folder-label-{:?}", right.entity_id());
            let badge = vcx.debug_bounds(selector.leak()).unwrap();
            vcx.simulate_click(badge.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
            workspace.read_with(vcx, |workspace, _| assert_eq!(workspace.active, 1));
            assert!(vcx.debug_bounds("project-folder-known-selected-1").is_some());
            let choice = vcx.debug_bounds("project-folder-known-0").unwrap();
            vcx.simulate_click(choice.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
            assert!(matches!(commands.try_recv().unwrap(), Command::CreateSession { working_dir: Some(directory), .. } if directory == alpha));
            workspace.read_with(vcx, |workspace, cx| {
                assert_eq!(workspace.slots.len(), 2);
                assert_eq!(workspace.active, 1);
                assert_eq!(workspace.slots[0].panel, left);
                assert_ne!(workspace.slots[1].panel, right);
                assert_eq!(left.read(cx).input.read(cx).content.as_ref(), "left draft");
                let switched = workspace.slots[1].panel.read(cx);
                assert_eq!(switched.working_dir.as_deref(), Some(alpha.as_str()));
                assert_eq!(switched.input.read(cx).content.as_ref(), "right draft");
                assert_eq!(switched.project_folder_label.as_deref(), Some("alpha"));
            });
        });
    }

    #[gpui::test]
    fn chinese_sidebar_keeps_unassigned_history_and_shows_registered_empty_projects(cx: &mut gpui::TestAppContext) {
        crate::locale::with_zh(|| {
            let directory = tempfile::tempdir().unwrap();
            let key = directory.path().to_string_lossy().into_owned();
            let (bridge, commands) = harness::spawn_recording();
            let (workspace, vcx) = cx.add_window_view(|_, cx| {
                let mut workspace = Workspace::for_test(learning::Coach::new(), cx);
                workspace.bridge = bridge;
                workspace.register_project(&key);
                let mut history = pending::draft_session_info("unassigned".into(), Some("Unassigned history".into()), None);
                history.transcript_bytes = Some(5);
                workspace.sessions = vec![history];
                workspace
            });
            vcx.run_until_parked();
            let recent = vcx.debug_bounds("sidebar-recent-session-unassigned").unwrap();
            let projects = vcx.debug_bounds("sidebar-projects-section").unwrap();
            assert!(recent.bottom() <= projects.top());
            assert!(vcx.debug_bounds("sidebar-collections-section").is_none());
            assert!(vcx.debug_bounds("sidebar-project-dock").is_none());
            assert!(vcx.debug_bounds("sidebar-session-title-0").is_none());
            assert!(vcx.debug_bounds("sidebar-project-new-0").is_some());
            assert!(vcx.debug_bounds("sidebar-project-menu-button-0").is_some());
            let project = vcx.debug_bounds("sidebar-project-0").unwrap();
            vcx.simulate_click(project.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
            assert!(matches!(commands.try_recv().unwrap(), Command::CreateSession { working_dir: Some(directory), .. } if directory == key));
            workspace.read_with(vcx, |workspace, _| assert_eq!(workspace.slots.len(), 1));
            assert!(vcx.debug_bounds("sidebar-section-0").is_none());

            workspace.update(vcx, |workspace, cx| {
                workspace.collections.store.create_with_id("fixture", "测试集合", None, None).unwrap();
                cx.notify();
            });
            vcx.run_until_parked();
            let project = vcx.debug_bounds("sidebar-project-0").unwrap();
            let collections = vcx.debug_bounds("sidebar-collections-section").unwrap();
            assert!(project.bottom() <= collections.top());
        });
    }


    #[gpui::test]
    fn project_creation_keeps_draft_after_validation_and_directory_errors(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("existing")).unwrap();
        let (workspace, vcx) = cx.add_window_view(|_, cx| Workspace::for_test(learning::Coach::new(), cx));
        vcx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.show_project_folder_picker(true, window, cx);
                workspace.folder_picker_dir = Some(root.path().to_owned());
            });
        });
        vcx.run_until_parked();

        for draft in [" bad/name ", " existing "] {
            workspace.update(vcx, |workspace, cx| {
                workspace.folder_search.as_ref().unwrap().update(cx, |input, cx| {
                    input.set_content(draft.to_owned(), cx);
                });
            });
            vcx.simulate_keystrokes("enter");
            vcx.run_until_parked();
            workspace.read_with(vcx, |workspace, cx| {
                let input = workspace.folder_search.as_ref().unwrap().read(cx);
                assert_eq!(input.content.as_ref(), draft);
                assert!(workspace.folder_picker_error.is_some());
                assert!(workspace.folder_picker_creates_project);
            });
            assert!(vcx.debug_bounds("folder-picker-error").is_some());
        }
    }
}
