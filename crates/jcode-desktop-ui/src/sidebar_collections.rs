//! Collections: user-created, named groups of sessions (issue #7).
//!
//! A collection is chosen by the user, never derived from a directory. Member
//! sessions are listed under their collection above the automatic project
//! groups, and leave their project group while they belong to one. The store
//! itself lives in the SDK (`~/.jcode/collections.json`), shared with every
//! window and client. Changes apply to the in-memory copy at once and are
//! written on a background thread, replayed against the file under its lock.
use super::*;
use jcode_sdk::collections::{Collection, Collections};

/// Sidebar group keys for collections never collide with project keys, which
/// are absolute paths, `ssh://` URLs, or empty.
pub(super) const KEY_PREFIX: &str = "collection:";

pub(super) fn key(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

pub(super) fn id_from_key(key: &str) -> Option<&str> {
    key.strip_prefix(KEY_PREFIX)
}

pub(super) const COLLECTION_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="black" stroke-width="1.6" stroke-linejoin="round"><rect x="3" y="8" width="18" height="12" rx="2.5"/><path d="M6 5h12M8.5 2.5h7"/></g></svg>"#;
const ADD_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round"/></svg>"#;
const REMOVE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12h14" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round"/></svg>"#;
const TRASH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

/// One persisted change, applied locally and then replayed on disk.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Change {
    Create {
        id: String,
        name: String,
        default_dir: Option<String>,
        instructions: Option<String>,
    },
    Edit {
        id: String,
        name: String,
        default_dir: Option<String>,
        instructions: Option<String>,
    },
    Delete(String),
    Assign {
        session_id: String,
        collection: Option<String>,
    },
    Reorder(String, isize),
}

impl Change {
    pub(super) fn apply(&self, store: &mut Collections) -> anyhow::Result<()> {
        match self.clone() {
            Change::Create {
                id,
                name,
                default_dir,
                instructions,
            } => store.create_with_id(&id, &name, default_dir, instructions),
            Change::Edit {
                id,
                name,
                default_dir,
                instructions,
            } => store.edit(&id, &name, default_dir, instructions),
            Change::Delete(id) => {
                store.delete(&id);
                Ok(())
            }
            Change::Assign {
                session_id,
                collection,
            } => store.assign(&session_id, collection.as_deref()),
            Change::Reorder(id, delta) => {
                store.reorder(&id, delta);
                Ok(())
            }
        }
    }
}

/// The inline editor for a new or existing collection.
pub(super) struct Form {
    /// `None` creates a collection.
    pub(super) editing: Option<String>,
    pub(super) name: Entity<PromptInput>,
    pub(super) directory: Entity<PromptInput>,
    pub(super) instructions: Entity<PromptInput>,
    /// Sessions to move into the new collection once it exists.
    pub(super) assign: Option<String>,
    pub(super) confirm_delete: bool,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) store: Collections,
    /// Where the store comes from. Tests and screenshots use a private file.
    path: Option<PathBuf>,
    pub(super) form: Option<Form>,
    pub(super) error: Option<String>,
    /// Session whose actions menu is open.
    pub(super) menu: Option<String>,
    menu_anchor: gpui::Point<gpui::Pixels>,
    /// Background writes, in order, so a later change never lands first.
    writes: Vec<gpui::Task<()>>,
    loaded: bool,
}

impl State {
    pub(super) fn path(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(store_path)
    }

    /// Point at a private store (tests and offline screenshots).
    #[cfg(test)]
    pub(super) fn with_path(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }

    pub(super) fn collection_of(&self, session_id: &str) -> Option<&Collection> {
        self.store.collection_of(session_id)
    }
}

/// The store follows `JCODE_HOME`, so screenshot and test runs that isolate it
/// never read or write the user's real collections.
fn store_path() -> PathBuf {
    jcode_sdk::collections::path()
}

fn optional(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

impl Workspace {
    /// Offline screenshot fixture: two collections spanning folders, an empty
    /// one, and ordinary project groups below. Writes only to the isolated
    /// `JCODE_HOME` the screenshot harness provides.
    pub(super) fn init_collections_fixture(&mut self) {
        if !harness::screenshot_mode()
            || std::env::var_os("JCODE_DESKTOP_SCREENSHOT_COLLECTIONS").is_none()
            || std::env::var_os("JCODE_HOME").is_none()
        {
            return;
        }
        let Some(template) = self.sessions.first().cloned() else {
            return;
        };
        let history = [
            (
                "collections-fixture-pricing",
                "Compare vendor pricing tiers",
                "/workspace/research",
                600_000,
            ),
            (
                "collections-fixture-survey",
                "Summarize customer survey",
                "/workspace/notes",
                3_600_000,
            ),
            (
                "collections-fixture-brief",
                "Draft the client X brief",
                "/workspace/clients/x",
                7_200_000,
            ),
            (
                "collections-fixture-invoice",
                "Reconcile March invoices",
                "/workspace/clients/x",
                86_400_000,
            ),
            (
                "collections-fixture-lint",
                "Fix flaky lint job",
                "/workspace/example",
                172_800_000,
            ),
        ];
        for (id, title, dir, age) in history {
            let mut session = template.clone();
            session.session_id = id.into();
            session.title = Some(title.into());
            session.working_dir = Some(dir.into());
            session.edit_stats = None;
            session.updated_at_ms = Some(unix_now_ms().saturating_sub(age));
            self.sessions.push(session);
        }
        let result = jcode_sdk::collections::update(|store| {
            let research = store.create("Q4 research", None, Some("Cite every source.".into()))?;
            let client = store.create("Client X", None, None)?;
            store.create("Reading list", None, None)?;
            store.assign("collections-fixture-pricing", Some(&research))?;
            store.assign("collections-fixture-survey", Some(&research))?;
            store.assign("collections-fixture-brief", Some(&client))?;
            store.assign("collections-fixture-invoice", Some(&client))?;
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("collections fixture failed: {error:#}");
        }
        // Visual review of the row menu without synthetic input.
        if std::env::var("JCODE_DESKTOP_SCREENSHOT_COLLECTIONS").as_deref() == Ok("menu") {
            self.collections.menu = Some("collections-fixture-lint".into());
            self.collections.menu_anchor = gpui::point(px(220.0), px(210.0));
        }
    }

    /// Visual review of the editor (needs a window to focus its inputs).
    pub(super) fn open_collections_fixture_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if harness::screenshot_mode()
            && std::env::var("JCODE_DESKTOP_SCREENSHOT_COLLECTIONS").as_deref() == Ok("form")
            && self.collections.form.is_none()
        {
            self.begin_new_collection(None, window, cx);
            if let Some(form) = &self.collections.form {
                let (name, instructions) = (form.name.clone(), form.instructions.clone());
                name.update(cx, |input, cx| input.set_content("Client X".into(), cx));
                instructions.update(cx, |input, cx| {
                    input.set_content("Answer in the client's voice.".into(), cx)
                });
            }
        }
    }

    /// Load collections once, off the UI thread.
    pub(super) fn load_collections(&mut self, cx: &mut Context<Self>) {
        if self.collections.loaded {
            return;
        }
        self.collections.loaded = true;
        let path = self.collections.path();
        let task = cx
            .background_executor()
            .spawn(async move { jcode_sdk::collections::load_from(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(store) => this.collections.store = store,
                    Err(error) => {
                        eprintln!("could not load collections: {error:#}");
                        this.collections.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Apply a change now and persist it in the background. A rejected change
    /// leaves memory untouched and reports why.
    pub(super) fn change_collections(&mut self, change: Change, cx: &mut Context<Self>) -> bool {
        let mut next = self.collections.store.clone();
        if let Err(error) = change.apply(&mut next) {
            self.collections.error = Some(format!("{error:#}"));
            cx.notify();
            return false;
        }
        self.collections.store = next;
        self.collections.error = None;
        let path = self.collections.path();
        let previous = std::mem::take(&mut self.collections.writes);
        let write = cx.background_executor().spawn(async move {
            for task in previous {
                task.await;
            }
            jcode_sdk::collections::update_at(&path, |store| change.apply(store))
                .map(|(store, ())| store)
        });
        self.collections
            .writes
            .push(cx.spawn(async move |this, cx| {
                let result = write.await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        // The file may hold edits from other windows too.
                        Ok(store) => this.collections.store = store,
                        Err(error) => {
                            eprintln!("could not save collections: {error:#}");
                            this.collections.error = Some(format!("{error:#}"));
                        }
                    }
                    cx.notify();
                });
            }));
        cx.notify();
        true
    }

    /// Wait for queued background writes (tests only).
    #[cfg(test)]
    pub(super) fn collection_writes(&mut self) -> Vec<gpui::Task<()>> {
        std::mem::take(&mut self.collections.writes)
    }

    pub(super) fn assign_session_collection(
        &mut self,
        session_id: &str,
        collection: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.collections.menu = None;
        if let Some(id) = &collection {
            self.sidebar_projects.collapsed.insert(key(id), false);
        }
        self.change_collections(
            Change::Assign {
                session_id: session_id.to_owned(),
                collection,
            },
            cx,
        );
    }

    fn form_input(
        &self,
        placeholder: &'static str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<PromptInput> {
        let weak = cx.weak_entity();
        let cancel = weak.clone();
        let input = cx.new(|cx| {
            let mut input = PromptInput::new(cx, placeholder, move |_, _, window, app| {
                let _ = weak.update(app, |this, cx| this.save_collection_form(window, cx));
            })
            .without_command_completion()
            .with_on_overlay_cancel(move |app| {
                let _ = cancel.update(app, |this, cx| this.close_collection_form(cx));
                true
            });
            input.set_content(text.to_owned(), cx);
            input
        });
        let _ = window;
        input
    }

    /// Open the editor for a new collection, optionally moving a session in.
    pub(super) fn begin_new_collection(
        &mut self,
        assign: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_collection_form(None, assign, window, cx);
    }

    pub(super) fn begin_edit_collection(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_collection_form(Some(id), None, window, cx);
    }

    fn open_collection_form(
        &mut self,
        editing: Option<String>,
        assign: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = editing
            .as_deref()
            .and_then(|id| self.collections.store.get(id))
            .cloned()
            .unwrap_or_default();
        let name = self.form_input(
            "e.g. Q4 research",
            &existing.name,
            window,
            cx,
        );
        let directory = self.form_input(
            "Optional, e.g. ~/work",
            existing.default_dir.as_deref().unwrap_or_default(),
            window,
            cx,
        );
        let instructions = self.form_input(
            "Optional",
            existing.instructions.as_deref().unwrap_or_default(),
            window,
            cx,
        );
        window.focus(&name.read(cx).focus_handle.clone(), cx);
        self.focus_pending = false;
        self.collections.menu = None;
        self.collections.error = None;
        self.collections.form = Some(Form {
            editing,
            name,
            directory,
            instructions,
            assign,
            confirm_delete: false,
        });
        cx.notify();
    }

    pub(super) fn close_collection_form(&mut self, cx: &mut Context<Self>) {
        if self.collections.form.take().is_some() {
            self.collections.error = None;
            self.focus_pending = true;
            cx.notify();
        }
    }

    pub(super) fn save_collection_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.collections.form else {
            return;
        };
        let name = form.name.read(cx).content.to_string();
        let default_dir = optional(&form.directory.read(cx).content);
        let instructions = optional(&form.instructions.read(cx).content);
        if let Some(dir) = default_dir.as_deref()
            && !Path::new(&expand_home(dir)).is_dir()
        {
            self.collections.error = Some(format!("\"{dir}\" is not a folder on this computer."));
            cx.notify();
            return;
        }
        let default_dir = default_dir.map(|dir| expand_home(&dir));
        let assign = form.assign.clone();
        let (change, id) = match form.editing.clone() {
            Some(id) => (
                Change::Edit {
                    id: id.clone(),
                    name,
                    default_dir,
                    instructions,
                },
                id,
            ),
            None => {
                let id = self.collections.store.new_id();
                (
                    Change::Create {
                        id: id.clone(),
                        name,
                        default_dir,
                        instructions,
                    },
                    id,
                )
            }
        };
        if !self.change_collections(change, cx) {
            return;
        }
        self.sidebar_projects.collapsed.insert(key(&id), false);
        if let Some(session_id) = assign {
            self.assign_session_collection(&session_id, Some(id), cx);
        }
        self.close_collection_form(cx);
    }

    pub(super) fn delete_collection(&mut self, id: String, cx: &mut Context<Self>) {
        self.sidebar_projects.collapsed.remove(&key(&id));
        self.change_collections(Change::Delete(id), cx);
        self.close_collection_form(cx);
    }

    /// Start a new chat inside a collection, in its default folder when set.
    pub(super) fn new_chat_in_collection(&mut self, id: String, cx: &mut Context<Self>) {
        let directory = self
            .collections
            .store
            .get(&id)
            .and_then(|c| c.default_dir.clone())
            .or_else(|| self.default_working_dir());
        self.sidebar_projects.collapsed.insert(key(&id), false);
        self.pending_collection_drafts.push((String::new(), id));
        self.focus_pending = true;
        self.open_local_draft(directory, cx);
        // `open_local_draft` mounted the newest draft as the active panel.
        let draft = self
            .slots
            .get(self.active)
            .map(|slot| slot.panel.read(cx).session_id.clone());
        match (draft, self.pending_collection_drafts.last_mut()) {
            (Some(draft), Some(pending)) if Panel::is_pending_session_id(&draft) => {
                pending.0 = draft;
            }
            _ => {
                self.pending_collection_drafts.pop();
            }
        }
    }

    /// A draft started inside a collection received its real session id.
    pub(super) fn resolve_collection_draft(
        &mut self,
        request_id: &str,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .pending_collection_drafts
            .iter()
            .position(|(draft, _)| draft == request_id)
        {
            let (_, collection) = self.pending_collection_drafts.remove(index);
            self.assign_session_collection(session_id, Some(collection), cx);
        }
    }

    /// Header for one collection group.
    pub(super) fn render_collection_header(
        &self,
        index: usize,
        project: &sidebar_projects::Project,
        count: usize,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let id = id_from_key(&project.key).unwrap_or_default().to_owned();
        let has_instructions = self
            .collections
            .store
            .get(&id)
            .is_some_and(|c| c.instructions.is_some());
        let tooltip = format!(
            "{} {} · {} chat{}{}",
            if collapsed { "Expand" } else { "Collapse" },
            project.label,
            count,
            if count == 1 { "" } else { "s" },
            if has_instructions {
                " · has instructions"
            } else {
                ""
            },
        );
        let key = project.key.clone();
        let group: &'static str = "sidebar-collection-header";
        let edit_id = id.clone();
        let new_id = id.clone();
        div()
            .id(("sidebar-project", index))
            .debug_selector(move || format!("sidebar-project-{index}"))
            .group(group)
            .relative()
            .mx_2()
            .mb_1()
            .pl_2()
            .pr_1()
            .h(px(24.0))
            .flex()
            .items_center()
            .gap_1()
            .rounded_full()
            .cursor_pointer()
            .hover(|el| el.bg(theme.TOOL_BG))
            .tooltip(move |_, cx| {
                cx.new(|_| remotes::HeaderTooltip(tooltip.clone().into()))
                    .into()
            })
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    let collapsed = this.sidebar_projects.collapsed(&key, false);
                    this.sidebar_projects
                        .collapsed
                        .insert(key.clone(), !collapsed);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.begin_edit_collection(edit_id.clone(), window, cx);
                }),
            )
            .child(
                div()
                    .w(px(12.0))
                    .flex_none()
                    .text_size(px(10.0))
                    .text_color(theme.TEXT_DIM)
                    .child(if collapsed { "›" } else { "⌄" }),
            )
            .child(sidebar_worktrees::icon(COLLECTION_ICON, 11.0))
            .child(
                div()
                    .flex_shrink(1.0)
                    .min_w_0()
                    .truncate()
                    .debug_selector(move || format!("sidebar-project-label-{index}"))
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.TEXT_DIM)
                    .child(project.label.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .w(px(16.0))
                    .text_right()
                    .text_size(px(10.0))
                    .text_color(theme.TEXT_DIM)
                    .child(count.to_string()),
            )
            .child(
                sidebar_worktrees::hover_actions(group, 24.0)
                    .child(
                        sidebar_worktrees::header_action(
                            format!("sidebar-collection-edit-{index}").into(),
                            "Edit collection: name, folder, instructions",
                            div().child("⋯").into_any_element(),
                            true,
                            group,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.begin_edit_collection(id.clone(), window, cx);
                            }),
                        ),
                    )
                    .child(
                        sidebar_worktrees::header_action(
                            format!("sidebar-collection-new-{index}").into(),
                            "New chat in this collection",
                            div().child("+").into_any_element(),
                            true,
                            group,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.new_chat_in_collection(new_id.clone(), cx);
                            }),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// Empty collections have no session row to carry their header, so they
    /// are listed above the thread list instead.
    pub(super) fn render_empty_collections(
        &self,
        empty: &[Collection],
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if empty.is_empty() {
            return None;
        }
        let theme = Theme::global();
        let group: &'static str = "sidebar-empty-collection";
        Some(
            div()
                .id("sidebar-empty-collections")
                .debug_selector(|| "sidebar-empty-collections".into())
                .flex_none()
                .pt_2()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(empty.iter().enumerate().map(|(index, collection)| {
                    let edit = collection.id.clone();
                    let new = collection.id.clone();
                    let right = collection.id.clone();
                    div()
                        .id(("sidebar-empty-collection", index))
                        .debug_selector(move || format!("sidebar-empty-collection-{index}"))
                        .group(group)
                        .relative()
                        .mx_2()
                        .pl_2()
                        .pr_1()
                        .h(px(24.0))
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|el| el.bg(theme.TOOL_BG))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.new_chat_in_collection(new.clone(), cx);
                            }),
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.begin_edit_collection(right.clone(), window, cx);
                            }),
                        )
                        .tooltip(|_, cx| {
                            cx.new(|_| {
                                remotes::HeaderTooltip(
                                    "Empty collection. Click to start a chat in it, or use a chat's ⋯ menu to move one here."
                                        .into(),
                                )
                            })
                            .into()
                        })
                        .child(div().w(px(12.0)).flex_none())
                        .child(sidebar_worktrees::icon(COLLECTION_ICON, 11.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.TEXT_DIM)
                                .child(collection.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(16.0))
                                .text_right()
                                .text_size(px(10.0))
                                .text_color(theme.TEXT_FAINT)
                                .child("0"),
                        )
                        .child(
                            sidebar_worktrees::hover_actions(group, 24.0).child(
                                sidebar_worktrees::header_action(
                                    format!("sidebar-empty-collection-edit-{index}").into(),
                                    "Edit collection: name, folder, instructions",
                                    div().child("⋯").into_any_element(),
                                    true,
                                    group,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.begin_edit_collection(edit.clone(), window, cx);
                                    }),
                                ),
                            ),
                        )
                }))
                .into_any_element(),
        )
    }

    pub(super) fn open_session_menu(
        &mut self,
        session_id: &str,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_gesture = None;
        self.project_menu.close();
        self.collections.menu = Some(session_id.to_owned());
        self.collections.menu_anchor = position;
        cx.notify();
    }

    pub(super) fn render_session_menu_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let session_id = self.collections.menu.as_deref()?;
        let position = self.collections.menu_anchor + gpui::point(px(10.0), px(4.0));
        Some(gpui::deferred(
            gpui::anchored().position(position).child(
                self.render_collection_menu(session_id, window.viewport_size().height, cx),
            ),
        ).with_priority(10).into_any_element())
    }

    fn render_collection_menu(
        &self,
        session_id: &str,
        viewport_height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let current = self
            .collections
            .collection_of(session_id)
            .map(|c| c.id.clone());
        use super::sidebar_project_menu::{popup_row as row, popup_separator, popup_surface};

        let mut menu = popup_surface("sidebar-collection-menu", viewport_height)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.collections.menu = None;
                cx.notify();
            }))
            .when(!self.collections.store.collections.is_empty(), |menu| {
                menu.child(div().flex_none().px(px(9.0)).pt_1().pb(px(3.0))
                    .text_size(px(10.0)).font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.TEXT_DIM).child(tr("Move to collection", "移到集合")))
            });
        for (index, collection) in self.collections.store.collections.iter().enumerate() {
            let chosen = current.as_deref() == Some(collection.id.as_str());
            let target = collection.id.clone();
            let session = session_id.to_owned();
            menu = menu.child(
                row(
                    ("sidebar-collection-choice", index).into(),
                    format!("sidebar-collection-choice-{index}"),
                    false,
                )
                .when(chosen, |el| el.bg(theme.ACCENT_DIM))
                .child(sidebar_worktrees::icon(COLLECTION_ICON, 14.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(collection.name.clone()),
                )
                .when(chosen, |el| {
                    el.child(div().flex_none().text_color(theme.TEXT_DIM).child("✓"))
                })
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.assign_session_collection(&session, Some(target.clone()), cx);
                    }),
                ),
            );
        }
        let session = session_id.to_owned();
        menu = menu.child(
            row(
                "sidebar-collection-create".into(),
                "sidebar-collection-create".into(),
                false,
            )
            .child(sidebar_worktrees::icon(ADD_ICON, 14.0))
            .child(tr("New collection…", "新建集合…"))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.begin_new_collection(Some(session.clone()), window, cx);
                }),
            ),
        );
        if current.is_some() {
            let session = session_id.to_owned();
            menu = menu.child(
                row(
                    "sidebar-collection-remove".into(),
                    "sidebar-collection-remove".into(),
                    false,
                )
                .child(sidebar_worktrees::icon(REMOVE_ICON, 14.0))
                .child(tr("Remove from collection", "从集合移除"))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.assign_session_collection(&session, None, cx);
                    }),
                ),
            );
        }
        let blocker = self.sidebar_archive_blocker(session_id, cx);
        let id = session_id.to_owned();
        let delete = row("sidebar-delete-session".into(), "sidebar-delete-session".into(), blocker.is_none())
            .child(gpui::svg().data(TRASH_ICON).size(px(14.0)).flex_none()
                .text_color(if blocker.is_some() { theme.TEXT_DIM } else { theme.ERROR }))
            .child(tr("Delete conversation…", "删除会话…"))
            .when_some(blocker, |el, reason| {
                el.cursor_default().text_color(theme.TEXT_DIM).tooltip(move |_, cx| {
                    cx.new(|_| remotes::HeaderTooltip(reason.into())).into()
                })
            })
            .when(blocker.is_none(), |el| el.on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.confirm_sidebar_archive(id.clone(), window, cx);
                }),
            ));
        menu.child(popup_separator()).child(delete).into_any_element()
    }

    /// Inline editor shown above the thread list.
    pub(super) fn render_collection_form(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let form = self.collections.form.as_ref()?;
        let theme = Theme::global();
        let editing = form.editing.clone();
        let label = |text: &'static str| {
            div()
                .text_size(px(10.0))
                .text_color(theme.TEXT_DIM)
                .child(text)
        };
        let pill = |id: &'static str, text: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .h(px(24.0))
                .px_3()
                .flex()
                .items_center()
                .rounded_full()
                .cursor_pointer()
                .text_size(px(11.0))
                .child(text)
        };
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                pill(
                    "collection-form-save",
                    if editing.is_some() { tr("Save", "保存") } else { tr("Create", "创建") },
                )
                .bg(theme.ACCENT)
                .text_color(theme.BG)
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.save_collection_form(window, cx);
                    }),
                ),
            )
            .child(
                pill("collection-form-cancel", tr("Cancel", "取消"))
                    .bg(theme.TOOL_BG)
                    .text_color(theme.TEXT_DIM)
                    .hover(|el| el.text_color(theme.TEXT))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.close_collection_form(cx);
                        }),
                    ),
            )
            .child(div().flex_1())
            .when_some(editing.clone(), |el, id| {
                let confirm = form.confirm_delete;
                el.child(
                    pill(
                        "collection-form-delete",
                        if confirm { tr("Really delete?", "确认删除？") } else { tr("Delete", "删除") },
                    )
                    .bg(if confirm {
                        theme.ERROR_BG
                    } else {
                        theme.TOOL_BG
                    })
                    .text_color(if confirm { theme.ERROR } else { theme.TEXT_DIM })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            let Some(form) = this.collections.form.as_mut() else {
                                return;
                            };
                            if form.confirm_delete {
                                this.delete_collection(id.clone(), cx);
                            } else {
                                form.confirm_delete = true;
                                cx.notify();
                            }
                        }),
                    ),
                )
            });
        let reorder = editing.as_ref().map(|id| {
            let up = id.clone();
            let down = id.clone();
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(label(tr("Order", "排序")))
                .child(
                    pill("collection-form-up", tr("Up", "上移"))
                        .h(px(20.0))
                        .px_2()
                        .bg(theme.TOOL_BG)
                        .text_color(theme.TEXT_DIM)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.change_collections(Change::Reorder(up.clone(), -1), cx);
                            }),
                        ),
                )
                .child(
                    pill("collection-form-down", tr("Down", "下移"))
                        .h(px(20.0))
                        .px_2()
                        .bg(theme.TOOL_BG)
                        .text_color(theme.TEXT_DIM)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.change_collections(Change::Reorder(down.clone(), 1), cx);
                            }),
                        ),
                )
        });
        Some(
            div()
                .id("collection-form")
                .debug_selector(|| "collection-form".into())
                .flex_none()
                .mx_2()
                .mt_2()
                .p_2()
                .rounded_xl()
                .bg(theme.PANEL_BG)
                .flex()
                .flex_col()
                .gap_2()
                .capture_action(cx.listener(|this, _: &crate::input::Submit, window, cx| {
                    cx.stop_propagation();
                    this.save_collection_form(window, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.0))
                        .text_color(theme.TEXT_DIM)
                        .child(sidebar_worktrees::icon(COLLECTION_ICON, 12.0))
                        .child(if editing.is_some() {
                            tr("Edit collection", "编辑集合")
                        } else {
                            tr("New collection", "新建集合")
                        }),
                )
                .child(label(tr("Name", "名称")))
                .child(form.name.clone())
                .child(label(tr("Default folder for new chats", "新会话的默认文件夹")))
                .child(form.directory.clone())
                .child(label(tr("Instructions", "指令")))
                .child(form.instructions.clone())
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme.TEXT_FAINT)
                        .child(tr("Instructions are sent privately with every message in this collection. Deleting a collection keeps its chats.", "本集合的指令会随每条消息一同发送。删除集合会保留其中的会话。")),
                )
                .children(reorder)
                .child(actions)
                .children(self.collections.error.clone().map(|error| {
                    div()
                        .id("collection-form-error")
                        .debug_selector(|| "collection-form-error".into())
                        .text_size(px(11.0))
                        .text_color(theme.ERROR)
                        .child(error)
                }))
                .into_any_element(),
        )
    }
}

/// `~/notes` style paths are accepted for convenience.
pub(super) fn expand_home(path: &str) -> String {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~")
        && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        && let Some(home) = crate::platform_dirs::home_dir()
    {
        return format!("{}{rest}", PathBuf::from(home).display());
    }
    path.to_owned()
}

#[cfg(test)]
#[path = "sidebar_collections_tests.rs"]
mod tests;
