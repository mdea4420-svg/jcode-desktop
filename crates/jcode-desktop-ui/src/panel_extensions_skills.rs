//! Extensions overlay — Skills tab: grouped skill list with toggles,
//! open-folder, trash-on-delete, plus the new/import forms.

use std::path::PathBuf;

use super::*;
use super::extensions::{Confirm, SkillForm, field_input, pill};
use crate::extensions::skills;

impl Panel {
    /// The Skills tab body: forms when open, else the grouped list.
    pub(super) fn render_extensions_skills(
        &mut self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        if state.skill_form.is_some() {
            return self.render_skill_form(cx);
        }
        let theme = Theme::global();
        let project_available = state.paths.project_dir.is_some();
        let matches = |skill: &skills::SkillEntry| {
            query.is_empty()
                || skill.name.to_lowercase().contains(query)
                || skill.description.to_lowercase().contains(query)
        };

        let mut groups: Vec<gpui::AnyElement> = Vec::new();
        groups.push(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill("skill-new", "skill-new".into(), "New skill", false)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.skill_form_open(cx)),
                        ),
                )
                .child(
                    pill("skill-import", "skill-import".into(), "Import…", false)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.skill_import_open(window, cx)),
                        ),
                )
                .when_some(state.skill_import.clone(), |el, field| {
                    el.child(div().w(px(260.)).child(field)).child(
                        pill(
                            "skill-import-apply",
                            "skill-import-apply".into(),
                            "Import",
                            false,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.skill_import_apply(cx)),
                        ),
                    )
                })
                .into_any_element(),
        );

        // Group entries by their source label, preserving first-seen order.
        let mut order: Vec<String> = Vec::new();
        for skill in &state.skills {
            let label = skill.source.label();
            if !order.contains(&label) {
                order.push(label);
            }
        }
        for label in order {
            let rows: Vec<&skills::SkillEntry> = state
                .skills
                .iter()
                .filter(|skill| skill.source.label() == label && matches(skill))
                .collect();
            if rows.is_empty() {
                continue;
            }
            groups.push(
                div()
                    .flex_none()
                    .mt_1()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.TEXT_DIM)
                    .child(format!("{label} · {}", rows.len()))
                    .into_any_element(),
            );
            for skill in rows {
                groups.push(self.render_skill_row(skill, cx));
            }
        }
        if state.skills.iter().filter(|s| matches(s)).count() == 0 {
            groups.push(
                div()
                    .id("extensions-skills-empty")
                    .debug_selector(|| "extensions-skills-empty".into())
                    .flex_none()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(theme.TEXT_DIM)
                    .child(if state.skills.is_empty() {
                        "No skills found. Create one above, or import a skill folder."
                    } else {
                        "Nothing matches the filter."
                    })
                    .into_any_element(),
            );
        }
        let _ = project_available;
        div()
            .id("extensions-skills-list")
            .debug_selector(|| "extensions-skills-list".into())
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .overflow_y_scroll()
            .track_scroll(&self.extensions.as_ref().unwrap().scroll)
            .children(groups)
            .into_any_element()
    }

    /// One skill row: name + description + shared hint + toggle/open/delete.
    fn render_skill_row(
        &self,
        skill: &skills::SkillEntry,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let name = skill.name.clone();
        let dir = skill.dir.clone();
        let enabled = skill.enabled;
        let editable = skill.editable;
        let shared = skill.shared;
        let _readonly = !editable;
        let readonly_label = match &skill.source {
            skills::SkillSource::Plugin(plugin) => Some(format!("Managed by plugin {plugin}")),
            skills::SkillSource::ClaudePlugin | skills::SkillSource::ProjectClaude => {
                Some("Claude Code".to_string())
            }
            _ => None,
        };
        let confirming_delete = self.extensions.as_ref().is_some_and(|state| {
            matches!(&state.confirm, Some(Confirm::SkillDelete { dir: d }) if d == &dir)
        });
        let confirming_toggle = self.extensions.as_ref().is_some_and(|state| {
            matches!(&state.confirm, Some(Confirm::SkillToggleShared { dir: d, .. }) if d == &dir)
        });

        let toggle_dir = dir.clone();
        let delete_dir = dir.clone();
        let cancel_dir = dir.clone();
        let open_dir = dir.clone();
        let key = dir.to_string_lossy().to_string();

        div()
            .id(format!("skill-row-{}", key.clone()))
            .debug_selector(move || format!("skill-row-{key}").into())
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_xl()
            .bg(theme.HEADER_BG)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .when(!enabled, |el| el.text_color(theme.TEXT_DIM))
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .child(if skill.description.is_empty() {
                                skill.dir.to_string_lossy().to_string()
                            } else {
                                skill.description.clone()
                            }),
                    )
                    .when(shared, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .text_color(theme.TEXT_FAINT)
                                .child("Shared with other tools"),
                        )
                    })
                    .when_some(readonly_label, |el, label| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .text_color(theme.TEXT_FAINT)
                                .child(label),
                        )
                    })
                    .when(editable, |el| {
                        el.child(
                            pill(
                                format!("skill-toggle-{}", toggle_dir.to_string_lossy().to_string()),
                                format!(
                                    "skill-toggle-{}",
                                    toggle_dir.file_name().unwrap().to_string_lossy()
                                ),
                                if enabled { "On" } else { "Off" },
                                enabled,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.skill_toggle(toggle_dir.clone(), !enabled, cx)
                                }),
                            ),
                        )
                    })
                    .child(
                        pill(
                            format!("skill-open-{}", open_dir.to_string_lossy().to_string()),
                            format!(
                                "skill-open-{}",
                                open_dir.file_name().unwrap().to_string_lossy()
                            ),
                            "Open folder",
                            false,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |_, _, _, cx| {
                                cx.open_url(&format!(
                                    "file://{}",
                                    percent_encode_path(&open_dir)
                                ));
                            }),
                        ),
                    )
                    .when(editable, |el| {
                        el.child(
                            pill(
                                format!("skill-delete-{}", delete_dir.to_string_lossy().to_string()),
                                format!(
                                    "skill-delete-{}",
                                    delete_dir.file_name().unwrap().to_string_lossy()
                                ),
                                "Delete",
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.extensions.as_mut() {
                                        state.confirm = Some(Confirm::SkillDelete {
                                            dir: delete_dir.clone(),
                                        });
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                    }),
            )
            .when(confirming_delete, |el| {
                let confirm_dir = dir.clone();
                let cancel_dir = cancel_dir.clone();
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .debug_selector(move || "skill-confirm-delete".into())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(format!(
                                    "Move `{}` to `~/.jcode/trash/skills`?",
                                    dir.file_name()
                                        .map(|name| name.to_string_lossy().to_string())
                                        .unwrap_or_default()
                                )),
                        )
                        .child(
                            pill(
                                format!("skill-confirm-{}", confirm_dir.to_string_lossy().to_string()),
                                "skill-confirm".to_string(),
                                "Delete",
                                true,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.skill_delete_confirm(confirm_dir.clone(), cx)
                                }),
                            ),
                        )
                        .child(
                            pill(
                                format!("skill-cancel-{}", cancel_dir.to_string_lossy().to_string()),
                                "skill-confirm-cancel".to_string(),
                                "Cancel",
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.extensions.as_mut() {
                                        state.confirm = None;
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                )
            })
            .when(confirming_toggle, |el| {
                let confirm_dir = dir.clone();
                let confirm_enabled = !enabled;
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .debug_selector(move || "skill-confirm-shared".into())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(
                                    "This skill is shared with other tools — changes affect them too.",
                                ),
                        )
                        .child(
                            pill(
                                format!("skill-shared-ok-{}", confirm_dir.to_string_lossy().to_string()),
                                "skill-shared-ok".to_string(),
                                "Continue",
                                true,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.skill_toggle_do(confirm_dir.clone(), confirm_enabled, cx)
                                }),
                            ),
                        )
                        .child(
                            pill(
                                format!("skill-shared-cancel-{}", dir.to_string_lossy().to_string()),
                                "skill-shared-cancel".to_string(),
                                "Cancel",
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.extensions.as_mut() {
                                        state.confirm = None;
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                )
            })
            .into_any_element()
    }

    /// Toggle a skill; shared (`~/.agents`) dirs ask first.
    fn skill_toggle(&mut self, dir: PathBuf, enabled: bool, cx: &mut Context<Self>) {
        let shared = self
            .extensions
            .as_ref()
            .and_then(|state| {
                state
                    .skills
                    .iter()
                    .find(|skill| skill.dir == dir)
                    .map(|skill| skill.shared)
            })
            .unwrap_or(false);
        if shared {
            if let Some(state) = self.extensions.as_mut() {
                state.confirm = Some(Confirm::SkillToggleShared {
                    dir: dir.clone(),
                });
            }
            cx.notify();
            return;
        }
        self.skill_toggle_do(dir, enabled, cx);
    }

    fn skill_toggle_do(&mut self, dir: PathBuf, enabled: bool, cx: &mut Context<Self>) {
        if let Some(state) = self.extensions.as_mut() {
            state.confirm = None;
        }
        let entry = self
            .extensions
            .as_ref()
            .and_then(|state| state.skills.iter().find(|skill| skill.dir == dir).cloned());
        let result = match entry {
            Some(entry) => skills::set_skill_enabled(&entry, enabled)
                .map(|_| format!("`{}`.", entry.name)),
            None => Err("Skill not found.".to_string()),
        };
        self.extensions_apply(
            result,
            if enabled { "Enabled" } else { "Disabled" },
            cx,
        );
    }

    fn skill_delete_confirm(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let (entry, jcode_home) = match self.extensions.as_ref() {
            Some(state) => (
                state
                    .skills
                    .iter()
                    .find(|skill| skill.dir == dir)
                    .cloned(),
                state.paths.jcode_home.clone(),
            ),
            None => (None, PathBuf::new()),
        };
        if let Some(state) = self.extensions.as_mut() {
            state.confirm = None;
        }
        let result = match entry {
            Some(entry) => skills::delete_skill(&entry, &jcode_home)
                .map(|target| format!("`{}` → `{}`.", entry.name, target.display())),
            None => Err("Skill not found.".to_string()),
        };
        self.extensions_apply(result, "Moved to trash:", cx);
    }

    // ------------------------------------------------------------- forms ---

    fn skill_form_open(&mut self, cx: &mut Context<Self>) {
        let name = Self::skill_field(cx, "name (kebab-case)", "");
        let description = Self::skill_field(cx, "description", "");
        if let Some(state) = self.extensions.as_mut() {
            state.skill_form = Some(SkillForm {
                name,
                description,
                global: true,
                error: None,
            });
        }
        cx.notify();
    }

    fn skill_field(
        cx: &mut Context<Self>,
        placeholder: &str,
        initial: &str,
    ) -> Entity<PromptInput> {
        let weak = cx.weak_entity();
        field_input(cx, placeholder, initial, move |app| {
            weak.update(app, |panel, cx| {
                if let Some(state) = panel.extensions.as_mut() {
                    state.skill_form = None;
                    state.skill_import = None;
                    cx.notify();
                }
            })
            .is_ok()
        })
    }

    fn render_skill_form(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        let Some(form) = state.skill_form.as_ref() else {
            return div().into_any_element();
        };
        let project_available = state.paths.project_dir.is_some();
        let global = form.global;
        let label = |text: &str| {
            div()
                .flex_none()
                .w(px(80.))
                .text_size(px(11.))
                .text_color(theme.TEXT_DIM)
                .child(text.to_string())
        };
        let row = |text: &str, content: gpui::AnyElement| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(label(text))
                .child(div().flex_1().min_w_0().child(content))
                .into_any_element()
        };
        let mut body: Vec<gpui::AnyElement> = vec![
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill("skill-form-back", "skill-form-back".into(), "Back", false)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                if let Some(state) = this.extensions.as_mut() {
                                    state.skill_form = None;
                                }
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("New skill"),
                )
                .into_any_element(),
            row("name", form.name.clone().into_any_element()),
            row("description", form.description.clone().into_any_element()),
        ];
        if project_available {
            body.push(row(
                "location",
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(
                        [("~/.jcode/skills", true), ("<project>/.jcode/skills", false)]
                            .into_iter()
                            .map(|(text, value)| {
                                pill(
                                    format!("skill-loc-{}", text),
                                    format!("skill-loc-{value}"),
                                    text,
                                    global == value,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        if let Some(form) = this
                                            .extensions
                                            .as_mut()
                                            .and_then(|s| s.skill_form.as_mut())
                                        {
                                            form.global = value;
                                        }
                                        cx.notify();
                                    }),
                                )
                                .into_any_element()
                            }),
                    )
                    .into_any_element(),
            ));
        }
        if let Some(error) = &form.error {
            body.push(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(theme.ERROR)
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        body.push(
            div()
                .flex_none()
                .child(
                    pill("skill-create", "skill-create".into(), "Create", true).on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.skill_form_save(cx)),
                    ),
                )
                .into_any_element(),
        );
        div()
            .id("extensions-skill-form")
            .debug_selector(|| "extensions-skill-form".into())
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .overflow_y_scroll()
            .track_scroll(&self.extensions.as_ref().unwrap().scroll)
            .children(body)
            .into_any_element()
    }

    fn skill_form_save(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.extensions.as_ref() else {
            return;
        };
        let Some(form) = state.skill_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).content.trim().to_string();
        let description = form.description.read(cx).content.trim().to_string();
        let root = if form.global || state.paths.project_dir.is_none() {
            state.paths.jcode_home.join("skills")
        } else {
            state
                .paths
                .project_dir
                .as_ref()
                .unwrap()
                .join(".jcode/skills")
        };
        match skills::create_skill(&root, &name, &description) {
            Ok(dir) => {
                if let Some(state) = self.extensions.as_mut() {
                    state.skill_form = None;
                }
                self.extensions_apply(Ok(dir.display().to_string()), "Created", cx);
            }
            Err(error) => {
                if let Some(form) = self
                    .extensions
                    .as_mut()
                    .and_then(|state| state.skill_form.as_mut())
                {
                    form.error = Some(error);
                }
                cx.notify();
            }
        }
    }

    /// `Import…`: ask for a directory; the text field stays as fallback for
    /// platforms without a folder picker.
    fn skill_import_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.extensions.as_mut() {
            if state.skill_import.is_none() {
                state.skill_import = Some(Self::skill_field(
                    cx,
                    "or paste the skill folder path",
                    "",
                ));
            }
        }
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a skill folder (with SKILL.md)".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(dir) = paths.into_iter().next()
            {
                let _ = this.update(cx, |panel, cx| panel.skill_import(dir, cx));
            }
        })
        .detach();
        cx.notify();
    }

    /// Import the path typed into the fallback field.
    fn skill_import_apply(&mut self, cx: &mut Context<Self>) {
        let path = self
            .extensions
            .as_ref()
            .and_then(|state| state.skill_import.as_ref())
            .map(|field| field.read(cx).content.trim().to_string())
            .unwrap_or_default();
        if path.is_empty() {
            return;
        }
        self.skill_import(PathBuf::from(path), cx);
    }

    /// Copy the picked folder into `~/.jcode/skills`.
    fn skill_import(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let root = self
            .extensions
            .as_ref()
            .map(|state| state.paths.jcode_home.join("skills"))
            .unwrap_or_default();
        match skills::import_skill(&dir, &root) {
            Ok(dest) => {
                if let Some(state) = self.extensions.as_mut() {
                    state.skill_import = None;
                }
                self.extensions_apply(Ok(dest.display().to_string()), "Imported", cx);
            }
            Err(error) => {
                if let Some(state) = self.extensions.as_mut() {
                    state.error = Some(error);
                }
                cx.notify();
            }
        }
    }
}

/// `file://` URL encoding (same rules as `percent_encode` in
/// `panel_special_commands.rs`, kept private there).
fn percent_encode_path(path: &std::path::Path) -> String {
    let mut result = String::new();
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            result.push(char::from(byte));
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}
