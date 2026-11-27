//! Extensions overlay — MCP tab: grouped server list, enable/test/edit
//! controls, and the add/edit form (replaces the list while open).

use std::path::PathBuf;

use super::*;
use super::extensions::{
    Confirm, McpForm, McpTarget, PairRow, field_input, pill,
};
use crate::extensions::mcp;
use crate::locale::{is_zh, tr};

/// `jcode mcp test` result deadline.
const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Run `jcode mcp test <name>` off the UI thread; 30 s timeout, killed on
/// expiry. Never logs env/header values — output goes to the result only.
fn run_mcp_test(name: String, cwd: Option<PathBuf>) -> mcp::McpTestResult {
    let exe = crate::platform::companion_executable("jcode");
    let mut command = std::process::Command::new(exe);
    command
        .args(["mcp", "test", &name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    crate::platform::no_console_window(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return mcp::McpTestResult {
                ok: false,
                server: None,
                tools: Vec::new(),
                error: Some(format!("Could not run `jcode mcp test`: {error}")),
            };
        }
    };
    let deadline = std::time::Instant::now() + TEST_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return mcp::McpTestResult {
                    ok: false,
                    server: None,
                    tools: Vec::new(),
                    error: Some("Timed out after 30 s.".into()),
                };
            }
            Err(error) => {
                return mcp::McpTestResult {
                    ok: false,
                    server: None,
                    tools: Vec::new(),
                    error: Some(format!("Test run failed: {error}")),
                };
            }
        }
    };
    use std::io::Read as _;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    mcp::parse_mcp_test_output(&stdout, &stderr, status.success())
}

fn mcp_source_label(source: mcp::McpSource) -> &'static str {
    use mcp::McpSource::*;
    let chinese = match source {
        JcodeGlobal => "jcode · ~/.jcode/mcp.json",
        ProjectJcode => "项目 · .jcode/mcp.json",
        ProjectMcp => "项目 · .mcp.json",
        ProjectClaude => "项目 · .claude/mcp.json",
        ClaudeUser => "Claude Code · 用户",
        ClaudeProject => "Claude Code · 项目",
    };
    tr(source.label(), chinese)
}

fn mcp_test_error(error: &str) -> String {
    if !is_zh() {
        return error.into();
    }
    if error == "Timed out after 30 s." {
        "测试在 30 秒后超时。".into()
    } else if let Some(detail) = error.strip_prefix("Could not run `jcode mcp test`: ") {
        format!("无法运行 `jcode mcp test`：{detail}")
    } else if let Some(detail) = error.strip_prefix("Test run failed: ") {
        format!("运行测试失败：{detail}")
    } else {
        format!("测试失败：{error}")
    }
}

impl Panel {
    /// The MCP tab body: form when editing/adding, else the grouped list.
    pub(super) fn render_extensions_mcp(
        &mut self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self
            .extensions
            .as_ref()
            .is_some_and(|state| state.mcp_form.is_some())
        {
            return self.render_mcp_form(cx);
        }
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        let theme = Theme::global();
        let matches = |entry: &mcp::McpEntry| {
            query.is_empty()
                || entry.name.to_lowercase().contains(query)
                || entry.command.to_lowercase().contains(query)
                || entry
                    .url
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(query)
        };
        let sources = [
            mcp::McpSource::JcodeGlobal,
            mcp::McpSource::ProjectJcode,
            mcp::McpSource::ProjectMcp,
            mcp::McpSource::ProjectClaude,
            mcp::McpSource::ClaudeUser,
            mcp::McpSource::ClaudeProject,
        ];
        let mut groups: Vec<gpui::AnyElement> = Vec::new();
        groups.push(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill(
                        "mcp-add",
                        "mcp-add".into(),
                        tr("Add server", "添加服务器"),
                        false,
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.mcp_form_open(None, cx)),
                    ),
                )
                .into_any_element(),
        );
        for notice in &state.mcp.notices {
            groups.push(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(theme.TEXT_FAINT)
                    .child(format!("{notice} {}", tr("— skipped", "— 已跳过")))
                    .into_any_element(),
            );
        }
        for source in sources {
            let rows: Vec<&mcp::McpEntry> = state
                .mcp
                .entries
                .iter()
                .filter(|entry| entry.source == source && matches(entry))
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
                    .child(format!("{} · {}", mcp_source_label(source), rows.len()))
                    .into_any_element(),
            );
            for entry in rows {
                groups.push(self.render_mcp_row(entry, cx));
            }
        }
        if state.mcp.entries.iter().filter(|e| matches(e)).count() == 0 {
            groups.push(
                div()
                    .id("extensions-mcp-empty")
                    .debug_selector(|| "extensions-mcp-empty".into())
                    .flex_none()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(theme.TEXT_DIM)
                    .child(if state.mcp.entries.is_empty() {
                        tr("No MCP servers configured. Add one above, or paste a config snippet.", "尚未配置 MCP 服务器。可以在上方添加服务器，或粘贴配置片段。")
                    } else {
                        tr("Nothing matches the filter.", "没有匹配的 MCP 服务器。")
                    })
                    .into_any_element(),
            );
        }
        div()
            .id("extensions-mcp-list")
            .debug_selector(|| "extensions-mcp-list".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .overflow_y_scroll()
            .track_scroll(&self.extensions.as_ref().unwrap().scroll)
            .children(groups)
            .into_any_element()
    }

    /// One server row: name + transport chip + one-line detail + action
    /// pills; the test result expands inline below it.
    fn render_mcp_row(
        &self,
        entry: &mcp::McpEntry,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let detail = if entry.transport == mcp::McpTransport::Stdio {
            let mut line = entry.command.clone();
            for arg in &entry.args {
                line.push(' ');
                line.push_str(arg);
            }
            if !entry.env.is_empty() {
                line.push_str(&format!(
                    "{}{}",
                    tr(" · env: ", " · 环境变量："),
                    entry
                        .env
                        .iter()
                        .map(|(key, _)| key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            line
        } else {
            entry.url.clone().unwrap_or_default()
        };
        let name = entry.name.clone();
        let path = entry.path.clone();
        let enabled = entry.enabled;
        let editable = entry.editable;
        let plugin = entry.plugin.clone();
        let readonly = plugin.is_some()
            || matches!(
                entry.source,
                mcp::McpSource::ClaudeUser
                    | mcp::McpSource::ClaudeProject
                    | mcp::McpSource::ProjectClaude
            );
        let readonly_label = plugin
            .map(|plugin| {
                if is_zh() { format!("由插件 {plugin} 管理") } else { format!("Managed by plugin {plugin}") }
            })
            .or(readonly.then(|| "Claude Code".to_string()));
        let test = self
            .extensions
            .as_ref()
            .and_then(|state| state.tests.get(&name).cloned());
        let testing = self
            .extensions
            .as_ref()
            .is_some_and(|state| state.testing.contains(&name));
        let confirming = self.extensions.as_ref().is_some_and(|state| {
            matches!(&state.confirm, Some(Confirm::McpDelete { name: n, path: p }) if n == &name && p == &path)
        });

        let toggle_name = name.clone();
        let toggle_path = path.clone();
        let test_name = name.clone();
        let edit_name = name.clone();
        let delete_name = name.clone();
        let delete_path = path.clone();
        let cancel_name = name.clone();
        let confirm_name = name.clone();
        let confirm_path = path.clone();

        div()
            .id(format!("mcp-row-{}", name.clone()))
            .debug_selector({
                let name = name.clone();
                move || format!("mcp-row-{name}").into()
            })
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
                            .max_w(px(200.))
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .when(!enabled, |el| el.text_color(theme.TEXT_DIM))
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px_2()
                            .h(px(18.))
                            .rounded_full()
                            .bg(theme.TOOL_BG)
                            .text_size(px(10.))
                            .text_color(theme.TEXT_DIM)
                            .child(entry.transport.as_str()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .child(detail),
                    )
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
                                format!("mcp-toggle-{}", toggle_name.clone()),
                                format!("mcp-toggle-{toggle_name}"),
                                if enabled { tr("On", "开启") } else { tr("Off", "关闭") },
                                enabled,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    let result = mcp::set_mcp_enabled(
                                        &toggle_path,
                                        &toggle_name,
                                        !enabled,
                                    )
                                    .map(|_| format!("`{toggle_name}`."));
                                    this.extensions_apply(
                                        result,
                                        if enabled { tr("Disabled", "已关闭") } else { tr("Enabled", "已开启") },
                                        cx,
                                    );
                                }),
                            ),
                        )
                    })
                    .child(
                        pill(
                            format!("mcp-test-{}", test_name.clone()),
                            format!("mcp-test-{test_name}"),
                            if testing { tr("Testing…", "测试中…") } else { tr("Test", "测试") },
                            false,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.mcp_test(test_name.clone(), cx);
                            }),
                        ),
                    )
                    .when(editable, |el| {
                        el.child(
                            pill(
                                format!("mcp-edit-{}", edit_name.clone()),
                                format!("mcp-edit-{edit_name}"),
                                tr("Edit", "编辑"),
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.mcp_form_open(Some(&edit_name), cx)
                                }),
                            ),
                        )
                        .child(
                            pill(
                                format!("mcp-delete-{}", delete_name.clone()),
                                format!("mcp-delete-{delete_name}"),
                                tr("Delete", "删除"),
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.extensions.as_mut() {
                                        state.confirm = Some(Confirm::McpDelete {
                                            name: delete_name.clone(),
                                            path: delete_path.clone(),
                                        });
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                    }),
            )
            .when(confirming, |el| {
                let confirm_dbg = confirm_name.clone();
                let confirm_label = confirm_name.clone();
                let confirm_del = confirm_name.clone();
                let confirm_del2 = confirm_name.clone();
                let confirm_del_path = confirm_path.clone();
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .debug_selector(move || format!("mcp-confirm-{confirm_dbg}").into())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(if is_zh() { format!("从配置中删除 `{confirm_label}`？") } else { format!("Delete `{confirm_label}` from its config?") }),
                        )
                        .child(
                            pill(
                                format!("mcp-confirm-delete-{}", confirm_del.clone()),
                                format!("mcp-confirm-delete-{confirm_del}"),
                                tr("Delete", "删除"),
                                true,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.extensions.as_mut() {
                                        state.confirm = None;
                                    }
                                    let result =
                                        mcp::delete_mcp(&confirm_del_path, &confirm_del2)
                                            .map(|_| format!("`{confirm_del2}`."));
                                    this.extensions_apply(result, tr("Deleted", "已删除"), cx);
                                }),
                            ),
                        )
                        .child(
                            pill(
                                format!("mcp-confirm-cancel-{}", cancel_name.clone()),
                                format!("mcp-confirm-cancel-{cancel_name}"),
                                tr("Cancel", "取消"),
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
            .when_some(test, |el, result| {
                el.child(if result.ok {
                    let mut row = div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_1()
                        .debug_selector(move || format!("mcp-test-result-{name}").into());
                    if let Some(server) = &result.server {
                        row = row.child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(format!("{server} ·")),
                        );
                    }
                    row.child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .child(format!("{} {}", result.tools.len(), tr("tools", "个工具"))),
                    )
                    .children(result.tools.iter().map(|tool| {
                        div()
                            .flex_none()
                            .px_2()
                            .h(px(18.))
                            .rounded_full()
                            .bg(theme.TOOL_BG)
                            .text_size(px(10.))
                            .text_color(theme.TEXT_DIM)
                            .child(tool.clone())
                    }))
                } else {
                    div()
                        .flex()
                        .debug_selector(move || format!("mcp-test-result-{name}").into())
                        .text_size(px(11.))
                        .text_color(theme.ERROR)
                        .child(
                            result
                                .error
                                .as_deref()
                                .map(mcp_test_error)
                                .unwrap_or_else(|| tr("Test failed.", "测试失败。").into()),
                        )
                })
            })
            .into_any_element()
    }

    /// Spawn `jcode mcp test <name>` on the background executor.
    fn mcp_test(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(state) = self.extensions.as_mut() else {
            return;
        };
        if !state.testing.insert(name.clone()) {
            return;
        }
        cx.notify();
        let cwd = self
            .working_dir
            .clone()
            .filter(|dir| !dir.trim().is_empty())
            .map(PathBuf::from);
        let task_name = name.clone();
        let task = cx
            .background_executor()
            .spawn(async move { run_mcp_test(task_name, cwd) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |panel, cx| {
                if let Some(state) = panel.extensions.as_mut() {
                    state.testing.remove(&name);
                    state.tests.insert(name, result);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -------------------------------------------------------------- form ----

    /// A field whose Escape backs out of the form — minted per call so one
    /// callback is never moved into two inputs.
    fn mcp_form_field(
        cx: &mut Context<Self>,
        placeholder: &str,
        initial: &str,
    ) -> Entity<PromptInput> {
        let weak = cx.weak_entity();
        field_input(cx, placeholder, initial, move |app| {
            weak.update(app, |panel, cx| {
                if let Some(state) = panel.extensions.as_mut() {
                    state.mcp_form = None;
                    cx.notify();
                }
            })
            .is_ok()
        })
    }

    /// Open the add/edit form. `edit` names an existing entry to pre-fill.
    pub(super) fn mcp_form_open(&mut self, edit: Option<&str>, cx: &mut Context<Self>) {
        let Some(state) = self.extensions.as_ref() else {
            return;
        };
        let entry = edit.and_then(|name| {
            state.mcp.entries.iter().find(|entry| entry.name == name)
        });
        let (editing, target) = match entry {
            Some(entry) => (
                Some((entry.path.clone(), entry.name.clone())),
                if entry.source == mcp::McpSource::JcodeGlobal {
                    McpTarget::Global
                } else {
                    McpTarget::Project
                },
            ),
            None => (None, McpTarget::Global),
        };
        let field = Self::mcp_form_field;
        let name = field(
            cx,
            tr("name (e.g. filesystem)", "名称（例如 filesystem）"),
            &entry.map(|e| e.name.clone()).unwrap_or_default(),
        );
        let command = field(
            cx,
            tr("command (e.g. npx)", "命令（例如 npx）"),
            &entry.map(|e| e.command.clone()).unwrap_or_default(),
        );
        let args = match entry {
            Some(entry) if !entry.args.is_empty() => entry
                .args
                .iter()
                .map(|arg| field(cx, tr("arg", "参数"), arg))
                .collect(),
            _ => vec![field(cx, tr("arg", "参数"), "")],
        };
        let pair_rows = |cx: &mut Context<Self>, pairs: &[(String, String)]| {
            if pairs.is_empty() {
                vec![PairRow {
                    key: field(cx, tr("KEY", "键"), ""),
                    value: field(cx, tr("value", "值"), ""),
                    hidden: None,
                }]
            } else {
                pairs
                    .iter()
                    .map(|(key, value)| PairRow {
                        key: field(cx, tr("KEY", "键"), key),
                        // Masked: the real value stays in `hidden` until
                        // the row's reveal pill is clicked.
                        value: field(cx, tr("value", "值"), ""),
                        hidden: Some(value.clone()),
                    })
                    .collect()
            }
        };
        let env = pair_rows(cx, &entry.map(|e| e.env.clone()).unwrap_or_default());
        let headers = pair_rows(
            cx,
            &entry.map(|e| e.headers.clone()).unwrap_or_default(),
        );
        let url = field(
            cx,
            "https://…",
            &entry
                .and_then(|e| e.url.clone())
                .unwrap_or_default(),
        );
        self.extensions.as_mut().unwrap().mcp_form = Some(McpForm {
            editing,
            target,
            name,
            transport: entry.map(|e| e.transport).unwrap_or_default(),
            command,
            args,
            env,
            url,
            headers,
            paste: None,
            error: None,
        });
        cx.notify();
    }

    /// Collect the form's pair rows: revealed rows use the field content,
    /// masked rows keep their hidden value.
    fn form_pairs(rows: &[PairRow], cx: &App) -> Vec<(String, String)> {
        rows.iter()
            .filter_map(|row| {
                let key = row.key.read(cx).content.trim().to_string();
                if key.is_empty() {
                    return None;
                }
                let value = match &row.hidden {
                    Some(hidden) => hidden.clone(),
                    None => row.value.read(cx).content.trim().to_string(),
                };
                Some((key, value))
            })
            .collect()
    }

    fn render_mcp_form(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        let Some(form) = state.mcp_form.as_ref() else {
            return div().into_any_element();
        };
        let editing = form.editing.is_some();
        let project_available = state.paths.project_dir.is_some();
        let paste_open = form.paste.is_some();

        let label = |text: &str| {
            div()
                .flex_none()
                .w(px(80.))
                .text_size(px(11.))
                .text_color(theme.TEXT_DIM)
                .child(text.to_string())
        };
        let row = |label_text: &str, content: gpui::AnyElement| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(label(label_text))
                .child(div().flex_1().min_w_0().child(content))
                .into_any_element()
        };

        let transport = form.transport;
        let transport_pill = |kind: mcp::McpTransport, cx: &mut Context<Self>| {
            pill(
                format!("mcp-type-{}", kind.as_str()),
                format!("mcp-type-{}", kind.as_str()),
                kind.as_str(),
                transport == kind,
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this
                        .extensions
                        .as_mut()
                        .and_then(|state| state.mcp_form.as_mut())
                    {
                        form.transport = kind;
                    }
                    cx.notify();
                }),
            )
            .into_any_element()
        };

        let mut body: Vec<gpui::AnyElement> = vec![
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill("mcp-form-back", "mcp-form-back".into(), tr("Back", "返回"), false)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                if let Some(state) = this.extensions.as_mut() {
                                    state.mcp_form = None;
                                }
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(if editing { tr("Edit server", "编辑服务器") } else { tr("Add server", "添加服务器") }),
                )
                .child(div().flex_1().min_w_0())
                .child(
                    pill(
                        "mcp-paste-toggle",
                        "mcp-paste-toggle".into(),
                        tr("Paste JSON", "粘贴 JSON"),
                        paste_open,
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            let weak = cx.weak_entity();
                            if let Some(form) = this
                                .extensions
                                .as_mut()
                                .and_then(|state| state.mcp_form.as_mut())
                            {
                                form.paste = if form.paste.is_some() {
                                    None
                                } else {
                                    Some(field_input(
                                        cx,
                                        r#"{"mcpServers":{"name":{…}}}"#,
                                        "",
                                        move |app| {
                                            weak.update(app, |panel, _| {
                                                if let Some(form) = panel
                                                    .extensions
                                                    .as_mut()
                                                    .and_then(|s| s.mcp_form.as_mut())
                                                {
                                                    form.paste = None;
                                                }
                                            })
                                            .is_ok()
                                        },
                                    ))
                                };
                            }
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
        ];

        if let Some(paste) = &form.paste {
            body.push(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(paste.clone()))
                    .child(
                        pill(
                            "mcp-paste-apply",
                            "mcp-paste-apply".into(),
                            tr("Apply", "应用"),
                            false,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.mcp_paste_apply(cx)),
                        ),
                    )
                    .into_any_element(),
            );
        }

        body.push(row(tr("name", "名称"), form.name.clone().into_any_element()));
        body.push(row(
            tr("type", "类型"),
            div()
                .flex()
                .items_center()
                .gap_1()
                .children([
                    transport_pill(mcp::McpTransport::Stdio, cx),
                    transport_pill(mcp::McpTransport::Http, cx),
                    transport_pill(mcp::McpTransport::Sse, cx),
                ])
                .into_any_element(),
        ));
        if !editing && project_available {
            let target = form.target;
            body.push(row(
                tr("file", "配置文件"),
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(
                        [
                            ("Global", tr("Global", "全局"), McpTarget::Global),
                            ("Project", tr("Project", "项目"), McpTarget::Project),
                        ]
                            .into_iter()
                            .map(|(slug, label, kind)| {
                                pill(
                                    format!("mcp-target-{}", slug),
                                    format!("mcp-target-{}", slug.to_lowercase()),
                                    label,
                                    target == kind,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        if let Some(form) = this
                                            .extensions
                                            .as_mut()
                                            .and_then(|s| s.mcp_form.as_mut())
                                        {
                                            form.target = kind;
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

        if transport == mcp::McpTransport::Stdio {
            body.push(row(tr("command", "命令"), form.command.clone().into_any_element()));
            for (index, arg) in form.args.iter().enumerate() {
                body.push(row(
                    if index == 0 { tr("args", "参数") } else { "" },
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(div().flex_1().min_w_0().child(arg.clone()))
                        .child(
                            pill(
                                format!("mcp-arg-remove-{}", index),
                                format!("mcp-arg-remove-{index}"),
                                "−",
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(form) = this
                                        .extensions
                                        .as_mut()
                                        .and_then(|s| s.mcp_form.as_mut())
                                        && form.args.len() > 1
                                    {
                                        form.args.remove(index);
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                        .into_any_element(),
                ));
            }
            body.push(
                div()
                    .flex_none()
                    .pl(px(88.))
                    .child(
                        pill("mcp-arg-add", "mcp-arg-add".into(), tr("+ arg", "+ 参数"), false)
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    let weak = cx.weak_entity();
                                    let field = field_input(cx, tr("arg", "参数"), "", move |app| {
                                        weak.update(app, |_, _| ()).is_ok()
                                    });
                                    if let Some(form) = this
                                        .extensions
                                        .as_mut()
                                        .and_then(|s| s.mcp_form.as_mut())
                                    {
                                        form.args.push(field);
                                    }
                                    cx.notify();
                                }),
                            ),
                    )
                    .into_any_element(),
            );
            body.extend(self.render_pair_rows("env", &form.env, cx));
        } else {
            body.push(row(tr("url", "URL"), form.url.clone().into_any_element()));
            body.extend(self.render_pair_rows("headers", &form.headers, cx));
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
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill("mcp-save", "mcp-save".into(), tr("Save", "保存"), true).on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.mcp_form_save(cx)),
                    ),
                )
                .into_any_element(),
        );

        div()
            .id("extensions-mcp-form")
            .debug_selector(|| "extensions-mcp-form".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .overflow_y_scroll()
            .track_scroll(&self.extensions.as_ref().unwrap().scroll)
            .children(body)
            .into_any_element()
    }

    /// Key/value rows for env/headers, each `KEY` + masked value + reveal
    /// and remove pills, plus an "+ env"/"+ header" add pill.
    fn render_pair_rows(
        &self,
        label: &'static str,
        rows: &[PairRow],
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let theme = Theme::global();
        let mut out = Vec::new();
        let env = label == "env";
        let display_label = if env { tr("env", "环境变量") } else { tr("headers", "请求头") };
        for (index, pair) in rows.iter().enumerate() {
            let masked = pair.hidden.is_some();
            let mut row = div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex_none()
                        .w(px(80.))
                        .text_size(px(11.))
                        .text_color(theme.TEXT_DIM)
                        .child(if index == 0 { display_label.to_string() } else { String::new() }),
                )
                .child(div().w(px(140.)).flex_none().child(pair.key.clone()))
                .child(div().flex_1().min_w_0().child(pair.value.clone()));
            if masked {
                row = row.child(
                    pill(
                        format!("mcp-reveal-{label}-{index}"),
                        format!("mcp-reveal-{label}-{index}"),
                        tr("reveal", "显示"),
                        false,
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(form) = this
                                .extensions
                                .as_mut()
                                .and_then(|s| s.mcp_form.as_mut())
                            {
                                let rows = if env { &mut form.env } else { &mut form.headers };
                                if let Some(row) = rows.get_mut(index)
                                    && let Some(hidden) = row.hidden.take()
                                {
                                    row.value.update(cx, |value, cx| {
                                        value.set_content(hidden, cx)
                                    });
                                }
                            }
                            cx.notify();
                        }),
                    ),
                );
            }
            out.push(
                row.child(
                    pill(
                        format!("mcp-pair-remove-{label}-{index}"),
                        format!("mcp-pair-remove-{label}-{index}"),
                        "−",
                        false,
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(form) = this
                                .extensions
                                .as_mut()
                                .and_then(|s| s.mcp_form.as_mut())
                            {
                                let rows = if env { &mut form.env } else { &mut form.headers };
                                if rows.len() > 1 {
                                    rows.remove(index);
                                }
                            }
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
            );
        }
        out.push(
            div()
                .flex_none()
                .pl(px(88.))
                .child(
                    pill(
                        format!("mcp-pair-add-{}", label),
                        format!("mcp-pair-add-{label}"),
                        format!("+ {display_label}"),
                        false,
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let row = PairRow {
                                key: Self::mcp_form_field(cx, tr("KEY", "键"), ""),
                                value: Self::mcp_form_field(cx, tr("value", "值"), ""),
                                hidden: None,
                            };
                            if let Some(form) = this
                                .extensions
                                .as_mut()
                                .and_then(|s| s.mcp_form.as_mut())
                            {
                                if env {
                                    form.env.push(row);
                                } else {
                                    form.headers.push(row);
                                }
                            }
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
        );
        out
    }

    /// `Apply` in Paste JSON mode: one unnamed draft fills the form;
    /// anything else upserts all entries to the target file.
    fn mcp_paste_apply(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.extensions.as_mut() else {
            return;
        };
        let paths = state.paths.clone();
        let Some(form) = state.mcp_form.as_mut() else {
            return;
        };
        let Some(paste) = &form.paste else {
            return;
        };
        let text = paste.read(cx).content.to_string();
        match mcp::parse_mcp_paste(&text) {
            Ok(mut parsed) if parsed.len() == 1 && parsed[0].0.is_empty() => {
                // Single bare server object → fill the form fields.
                let (_, draft) = parsed.remove(0);
                Self::fill_form_from_draft(form, &draft, cx);
                form.paste = None;
                form.error = None;
            }
            Ok(parsed) => {
                let (target, editing) = (form.target, form.editing.clone());
                let path = Self::mcp_target_path_of(&paths, target, editing.as_ref());
                let mut imported = 0usize;
                let mut last_error = None;
                for (name, draft) in parsed {
                    let name = if name.is_empty() {
                        form.name.read(cx).content.trim().to_string()
                    } else {
                        name
                    };
                    match mcp::upsert_mcp(&path, None, &name, &draft) {
                        Ok(()) => imported += 1,
                        Err(error) => last_error = Some(error),
                    }
                }
                if let Some(state) = self.extensions.as_mut() {
                    if let Some(form) = state.mcp_form.as_mut() {
                        if let Some(error) = last_error {
                            form.error = Some(if is_zh() {
                                format!("已导入 {imported} 个服务器。最后一个错误：{error}")
                            } else {
                                format!("Imported {imported}; last error: {error}")
                            });
                            cx.notify();
                            return;
                        }
                    }
                    state.mcp_form = None;
                }
                self.extensions_apply(
                    Ok(format!("{imported} {}", tr("server(s).", "个服务器。"))),
                    tr("Imported", "已导入"),
                    cx,
                );
                return;
            }
            Err(error) => {
                form.error = Some(if is_zh() { format!("无法解析或导入 MCP 配置：{error}") } else { error });
            }
        }
        cx.notify();
    }

    /// Fill an open form from a parsed draft (single-object paste).
    fn fill_form_from_draft(form: &mut McpForm, draft: &mcp::McpDraft, cx: &mut Context<Self>) {
        form.transport = draft.transport;
        form.command.update(cx, |field, cx| {
            field.set_content(draft.command.clone(), cx)
        });
        form.url.update(cx, |field, cx| {
            field.set_content(draft.url.clone(), cx)
        });
        // Replace arg/pair rows with the draft's values.
        let mut args: Vec<Entity<PromptInput>> = Vec::new();
        for arg in &draft.args {
            args.push(Self::mcp_form_field(cx, tr("arg", "参数"), arg));
        }
        if args.is_empty() {
            args.push(Self::mcp_form_field(cx, tr("arg", "参数"), ""));
        }
        form.args = args;
        form.env = Vec::new();
        for (key, value) in &draft.env {
            form.env.push(PairRow {
                key: Self::mcp_form_field(cx, tr("KEY", "键"), key),
                value: Self::mcp_form_field(cx, tr("value", "值"), value),
                hidden: None,
            });
        }
        if form.env.is_empty() {
            form.env.push(PairRow {
                key: Self::mcp_form_field(cx, tr("KEY", "键"), ""),
                value: Self::mcp_form_field(cx, tr("value", "值"), ""),
                hidden: None,
            });
        }
        form.headers = Vec::new();
        for (key, value) in &draft.headers {
            form.headers.push(PairRow {
                key: Self::mcp_form_field(cx, tr("Key", "键"), key),
                value: Self::mcp_form_field(cx, tr("value", "值"), value),
                hidden: None,
            });
        }
        if form.headers.is_empty() {
            form.headers.push(PairRow {
                key: Self::mcp_form_field(cx, tr("Key", "键"), ""),
                value: Self::mcp_form_field(cx, tr("value", "值"), ""),
                hidden: None,
            });
        }
    }

    /// The file a save/import writes to.
    fn mcp_target_path_of(
        paths: &crate::extensions::ExtensionPaths,
        target: McpTarget,
        editing: Option<&(PathBuf, String)>,
    ) -> PathBuf {
        if let Some((path, _)) = editing {
            return path.clone();
        }
        match target {
            McpTarget::Global => paths.jcode_home.join("mcp.json"),
            McpTarget::Project => paths
                .project_dir
                .as_ref()
                .map(|project| project.join(".jcode/mcp.json"))
                .unwrap_or_else(|| paths.jcode_home.join("mcp.json")),
        }
    }

    /// Validate and write the form via `upsert_mcp`.
    fn mcp_form_save(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.extensions.as_ref() else {
            return;
        };
        let Some(form) = state.mcp_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).content.trim().to_string();
        let editing_name = form.editing.as_ref().map(|(_, name)| name.clone());
        let error = if name.is_empty() {
            Some(tr("Server name is required.", "请输入服务器名称。").to_string())
        } else if editing_name.as_deref() != Some(name.as_str())
            && state.mcp.entries.iter().any(|entry| {
                entry.name == name
                    && entry.path
                        == Self::mcp_target_path_of(&state.paths, form.target, form.editing.as_ref())
            })
        {
            Some(if is_zh() {
                format!("该配置中已存在名为 `{name}` 的服务器。")
            } else {
                format!("A server named `{name}` already exists there.")
            })
        } else if form.transport == mcp::McpTransport::Stdio
            && form.command.read(cx).content.trim().is_empty()
        {
            Some(tr("stdio servers need a command.", "stdio 服务器需要启动命令。").to_string())
        } else if form.transport != mcp::McpTransport::Stdio
            && form.url.read(cx).content.trim().is_empty()
        {
            Some(tr("http/sse servers need a url.", "HTTP 或 SSE 服务器需要 URL。").to_string())
        } else {
            None
        };
        if let Some(error) = error {
            if let Some(form) = self
                .extensions
                .as_mut()
                .and_then(|state| state.mcp_form.as_mut())
            {
                form.error = Some(error);
            }
            cx.notify();
            return;
        }
        let draft = mcp::McpDraft {
            transport: form.transport,
            command: form.command.read(cx).content.trim().to_string(),
            args: form
                .args
                .iter()
                .map(|field| field.read(cx).content.trim().to_string())
                .filter(|arg| !arg.is_empty())
                .collect(),
            env: Self::form_pairs(&form.env, cx),
            url: form.url.read(cx).content.trim().to_string(),
            headers: Self::form_pairs(&form.headers, cx),
        };
        let path = Self::mcp_target_path_of(&state.paths, form.target, form.editing.as_ref());
        let original = form.editing.as_ref().map(|(_, name)| name.as_str());
        let saved_name = name.clone();
        let result = mcp::upsert_mcp(&path, original, &name, &draft)
            .map(|_| format!("`{saved_name}`."));
        if let Some(state) = self.extensions.as_mut()
            && result.is_ok()
        {
            state.mcp_form = None;
        }
        self.extensions_apply(result, tr("Saved", "已保存"), cx);
    }
}
