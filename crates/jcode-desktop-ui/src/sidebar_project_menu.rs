use super::*;
use std::io::Write as _;

#[derive(Clone, Default, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub(super) name: Option<String>,
    pub(super) pinned: bool,
    pub(super) archived: bool,
    pub(super) removed: bool,
    #[serde(default)]
    pub(super) registered: bool,
}

#[derive(Default)]
pub(super) struct State {
    entries: Option<HashMap<String, Preferences>>,
    pub(super) open: Option<String>,
    pub(super) anchor: gpui::Point<gpui::Pixels>,
    editing: Option<Entity<PromptInput>>,
    confirm_remove: bool,
    pub(super) show_removed: bool,
    error: Option<String>,
}

fn preference_path() -> Option<PathBuf> {
    #[cfg(test)]
    { None }
    #[cfg(not(test))]
    { crate::learning::state_path().map(|path| path.with_file_name("desktop-projects.json")) }
}

fn read_preferences(path: &Path) -> std::io::Result<HashMap<String, Preferences>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(error),
    }
}

fn write_preferences(path: &Path, entries: &HashMap<String, Preferences>) -> std::io::Result<()> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(file.as_file_mut(), entries)?;
    file.flush()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

impl State {
    pub(super) fn load(&mut self) {
        self.entries.get_or_insert_with(|| {
            preference_path().and_then(|path| read_preferences(&path).ok()).unwrap_or_default()
        });
    }

    pub(super) fn get(&self, key: &str) -> Option<&Preferences> {
        self.entries.as_ref()?.get(key)
    }

    pub(super) fn has_removed(&self) -> bool {
        self.entries.as_ref().is_some_and(|entries| entries.values().any(|p| p.removed))
    }

    fn update(&mut self, key: &str, edit: impl FnOnce(&mut Preferences)) -> std::io::Result<()> {
        static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = WRITE_LOCK.lock().map_err(|_| std::io::Error::other("project settings lock poisoned"))?;
        let path = preference_path();
        let mut entries = match &path {
            Some(path) => read_preferences(path)?,
            #[cfg(test)]
            None => self.entries.clone().unwrap_or_default(),
            #[cfg(not(test))]
            None => return Err(std::io::Error::other("project settings directory unavailable")),
        };
        let entry = entries.entry(key.to_owned()).or_default();
        edit(entry);
        if *entry == Preferences::default() {
            entries.remove(key);
        }
        if let Some(path) = path {
            write_preferences(&path, &entries)?;
        }
        self.entries = Some(entries);
        Ok(())
    }

    pub(super) fn close(&mut self) {
        self.open = None;
        self.editing = None;
        self.confirm_remove = false;
        self.error = None;
    }
}

pub(super) fn popup_surface(id: &'static str, viewport_height: gpui::Pixels) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div().id(id).debug_selector(move || id.into()).occlude()
        .w(px(192.0)).max_h((viewport_height - px(24.0)).max(px(1.0)))
        .overflow_y_scroll().p(px(5.0)).flex().flex_col().gap(px(1.0))
        .rounded(px(10.0)).border_1().border_color(theme.PANEL_BORDER)
        .bg(theme.PANEL_BG).shadow_md()
}

pub(super) fn popup_row(id: gpui::ElementId, selector: String, dangerous: bool) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div().id(id).debug_selector(move || selector.clone())
        .h(px(28.0)).flex_none().px(px(9.0)).flex().items_center().gap(px(8.0))
        .rounded(px(6.0)).text_size(px(11.5)).cursor_pointer()
        .text_color(if dangerous { theme.ERROR } else { theme.TEXT })
        .hover(move |el| el.bg(if dangerous { theme.ERROR_BG } else { theme.TOOL_BG }))
}

pub(super) fn popup_separator() -> gpui::Div {
    div().flex_none().mx(px(7.0)).my(px(3.0)).border_t_1().border_color(Theme::global().PANEL_BORDER)
}

fn menu_row(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    popup_row(id.into(), id.into(), false).child(label)
}

impl Workspace {
    pub(super) fn register_project(&mut self, path_or_key: &str) -> String {
        let path = Path::new(path_or_key);
        let key = sidebar_projects::git_root(path)
            .unwrap_or_else(|| path.to_path_buf())
            .to_string_lossy().into_owned();
        if path.is_absolute()
            && let Err(error) = self.project_menu.update(&key, |p| {
                p.registered = true;
                p.removed = false;
            })
        {
            eprintln!("{} {error}", tr("Could not save project settings.", "无法保存项目设置。"));
        }
        key
    }

    pub(super) fn registered_sidebar_projects(&self) -> Vec<sidebar_projects::Project> {
        let mut projects = self.project_menu.entries.iter().flatten()
            .filter(|(key, p)| p.registered && (!p.removed || self.project_menu.show_removed)
                && Path::new(key.as_str()).is_absolute())
            .map(|(key, p)| sidebar_projects::Project {
                key: key.clone(),
                label: p.name.clone().unwrap_or_else(|| sidebar_projects::path_label(key)),
                location: Some(compact_working_dir(key)),
            }).collect::<Vec<_>>();
        projects.sort_by_cached_key(|p| (p.label.to_lowercase(), p.key.clone()));
        projects
    }


    pub(super) fn render_removed_projects_toggle(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let label = if self.project_menu.show_removed {
            tr("Hide removed projects", "隐藏已移除项目")
        } else {
            tr("Show removed projects", "查看已移除项目")
        };
        menu_row("sidebar-removed-projects", label).mx_2().flex_none()
            .text_color(Theme::global().TEXT_DIM)
            .on_click(cx.listener(|this, _, _, cx| {
                this.project_menu.close();
                this.project_menu.show_removed = !this.project_menu.show_removed;
                cx.notify();
            })).into_any_element()
    }

    pub(super) fn toggle_project_menu(
        &mut self, key: &str, anchor: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>,
    ) {
        let was_open = self.project_menu.open.as_deref() == Some(key);
        self.project_menu.close();
        self.collections.menu = None;
        if !was_open {
            self.project_menu.open = Some(key.to_owned());
            self.project_menu.anchor = anchor;
        }
        cx.notify();
    }

    fn change_project(&mut self, key: &str, edit: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        match self.project_menu.update(key, edit) {
            Ok(()) => {
                self.project_menu.close();
                self.focus_pending = true;
            }
            Err(error) => self.project_menu.error = Some(format!("{} {error}", tr("Could not save project settings.", "无法保存项目设置。"))),
        }
        cx.notify();
    }

    pub(super) fn render_project_menu(
        &self, project: &sidebar_projects::Project, viewport_height: gpui::Pixels, cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let key = &project.key;
        let prefs = self.project_menu.get(key).cloned().unwrap_or_default();
        let mut menu = popup_surface("sidebar-project-menu", viewport_height);
        if let Some(input) = &self.project_menu.editing {
            let input = input.clone();
            let key = key.clone();
            menu = menu.child(div().p_1().child(input.clone())).child(
                menu_row("project-save-name", tr("Save project name", "保存项目名称"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let name = input.read(cx).content.trim().to_owned();
                        this.change_project(&key, |p| {
                            p.name = (!name.is_empty()).then_some(name);
                            p.registered = true;
                            p.removed = false;
                        }, cx);
                    }))
            );
        } else if self.project_menu.confirm_remove {
            let key = key.clone();
            menu = menu.child(div().p_2().text_size(px(11.0)).child(tr(
                "Hide this project and its sessions from the sidebar? Files and conversation history are kept.",
                "将此项目及其会话移出侧栏？项目文件和会话记录会保留。",
            ))).child(menu_row("project-confirm-remove", tr("Remove project from sidebar", "确认移出侧栏"))
                .text_color(theme.ERROR)
                .on_click(cx.listener(move |this, _, _, cx| this.change_project(&key, |p| p.removed = true, cx))));
        } else if prefs.removed {
            let key = key.clone();
            menu = menu.child(menu_row("project-restore-removed", tr("Restore to sidebar", "恢复到侧栏"))
                .on_click(cx.listener(move |this, _, _, cx| this.change_project(&key, |p| p.removed = false, cx))));
        } else {
            let folder = key.clone();
            let new_folder = key.clone();
            let label = project.label.clone();
            let copy_label = label.clone();
            let pin_key = key.clone();
            let archive_key = key.clone();
            let copy_key = key.clone();
            let edit_key = key.clone();
            menu = menu
                .child(menu_row("project-open-folder", tr("Open folder", "打开文件夹"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Err(error) = open_project_directory(Path::new(&folder)) {
                            this.project_menu.error = Some(format!("{} {error}", tr("Could not open folder.", "无法打开文件夹。")));
                            cx.notify();
                        } else {
                            this.register_project(&folder);
                            this.project_menu.close();
                            cx.notify();
                        }
                    })))
                .child(menu_row("project-new-session", tr("New project session", "新建项目会话"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project_menu.close();
                        this.activate_or_open_project_directory(new_folder.clone(), cx);
                    })))
                .child(menu_row("project-edit-name", tr("Edit project name", "编辑项目名称"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let workspace = cx.weak_entity();
                        let key = edit_key.clone();
                        let input = cx.new(|cx| {
                            let mut input = PromptInput::new(cx, tr("Project name", "项目名称"), move |name, _, _, app| {
                                let _ = workspace.update(app, |this, cx| {
                                    let name = name.trim().to_owned();
                                    this.change_project(&key, |p| {
                                        p.name = (!name.is_empty()).then_some(name);
                                        p.registered = true;
                                        p.removed = false;
                                    }, cx);
                                });
                            }).without_command_completion();
                            input.set_content(label.clone(), cx);
                            input
                        });
                        window.focus(&input.read(cx).focus_handle.clone(), cx);
                        this.project_menu.editing = Some(input);
                        cx.notify();
                    })))
                .child(menu_row("project-pin", if prefs.pinned { tr("Unpin project", "取消置顶") } else { tr("Pin project", "置顶项目") })
                    .on_click(cx.listener(move |this, _, _, cx| this.change_project(&pin_key, |p| {
                        p.pinned = !p.pinned;
                        p.registered |= p.pinned;
                        p.removed = false;
                    }, cx))))
                .child(menu_row("project-archive", if prefs.archived { tr("Restore archived project", "恢复归档项目") } else { tr("Archive project", "归档项目") })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar_projects.collapsed.remove(&archive_key);
                        this.change_project(&archive_key, |p| p.archived = !p.archived, cx);
                    })))
                .child(menu_row("project-copy-path", tr("Copy project path", "复制项目路径"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_key.clone()));
                        this.project_menu.close();
                        cx.notify();
                    })));
            menu = menu.child(menu_row("project-copy-name", tr("Copy project name", "复制项目名称"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_label.clone()));
                    this.project_menu.close();
                    cx.notify();
                })));
            if Path::new(key).join(".git").exists() {
                let directory = key.clone();
                menu = menu.child(menu_row("project-git-worktree", tr("New worktree branch…", "新建工作树分支…"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.project_menu.close();
                        this.begin_worktree(directory.clone(), window, cx);
                    })));
            }
            menu = menu.child(popup_separator()).child(
                popup_row("project-remove".into(), "project-remove".into(), true)
                    .child(tr("Remove from sidebar…", "移出侧栏…"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu.confirm_remove = true;
                    cx.notify();
                })));
        }
        if let Some(error) = &self.project_menu.error {
            menu = menu.child(div().p_2().text_size(px(11.0)).text_color(theme.ERROR).child(error.clone()));
        }
        menu.child(menu_row("project-menu-close", tr("Close menu", "关闭菜单"))
            .text_color(theme.TEXT_DIM)
            .on_click(cx.listener(|this, _, _, cx| {
                this.project_menu.close();
                this.focus_pending = true;
                cx.notify();
            }))).into_any_element()
    }
}

fn open_project_directory(path: &Path) -> std::io::Result<()> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(std::io::ErrorKind::NotFound.into());
    }
    #[cfg(target_os = "windows")]
    let program = "explorer.exe";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(path).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn register_project_uses_git_root_and_restores_removed_projects(cx: &mut gpui::TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let child = root.join("src");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        let (workspace, vcx) = cx.add_window_view(|_, cx| Workspace::for_test(learning::Coach::new(), cx));
        workspace.update(vcx, |workspace, _| {
            let key = workspace.register_project(&child.to_string_lossy());
            assert_eq!(Path::new(&key), root.as_path());
            workspace.project_menu.update(&key, |p| p.removed = true).unwrap();
            assert!(workspace.registered_sidebar_projects().is_empty());
            workspace.project_menu.show_removed = true;
            assert_eq!(workspace.registered_sidebar_projects()[0].key, key);
            workspace.project_menu.show_removed = false;
            assert_eq!(workspace.register_project(&child.to_string_lossy()), key);
            assert_eq!(workspace.registered_sidebar_projects()[0].key, key);
        });
    }

    #[test]
    fn project_preferences_round_trip_and_replace() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        assert!(read_preferences(&path).unwrap().is_empty());
        let legacy: Preferences = serde_json::from_str(r#"{"pinned":true}"#).unwrap();
        assert!(!legacy.registered);
        let mut entries = HashMap::from([("project".into(), Preferences {
            name: Some("示例项目".into()), pinned: true, archived: false, removed: false, registered: true,
        })]);
        write_preferences(&path, &entries).unwrap();
        assert_eq!(read_preferences(&path).unwrap(), entries);
        entries.get_mut("project").unwrap().removed = true;
        write_preferences(&path, &entries).unwrap();
        assert_eq!(read_preferences(&path).unwrap(), entries);
    }
}
