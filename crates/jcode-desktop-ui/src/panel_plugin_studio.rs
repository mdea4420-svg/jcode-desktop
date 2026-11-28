//! In-app Plugin Studio:
//! 1. **AI Plugin Builder** (`AiBuilder`) — describe what you want (or pick a
//!    starter template) so the Jcode Agent authors, validates, and enables a
//!    new or existing plugin for itself in `~/.jcode/plugins/`.
//! 2. **Multi-File Plugin Code Editor** (`CodeEditor`) — browse, create, and
//!    edit `index.mjs`, `plugin.json`, `commands/*.md`, `hooks/hooks.json`, or
//!    `.mcp.json` directly inside Desktop with snippet insertion and one-click
//!    live validation + hot-reload.

use std::path::{Path, PathBuf};

use super::*;
use super::extensions::{field_input, pill, soft_wrap_text};
use crate::locale::{is_zh, tr};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AiPluginArch {
    Auto,
    Code,
    Declarative,
}

impl AiPluginArch {
    fn hint(self) -> &'static str {
        match self {
            Self::Auto => "根据需求自动选择（涉及侧栏面板、动态逻辑或自定义工具时优先用 Node ESM `index.mjs`，纯提示词模板用声明式）",
            Self::Code => "Node ESM 代码插件（`.claude-plugin/plugin.json` 中声明 `\"main\": \"index.mjs\"` + `jcode.d.ts`）",
            Self::Declarative => "声明式插件（`commands/*.md`、`skills/*/SKILL.md`、`hooks/hooks.json`、`.mcp.json`）",
        }
    }
}

pub(super) enum PluginStudio {
    AiBuilder {
        existing: Option<(String, PathBuf)>,
        name: Entity<PromptInput>,
        arch: AiPluginArch,
        requirement: Entity<PromptInput>,
        error: Option<String>,
    },
    CodeEditor {
        plugin_name: String,
        plugin_root: PathBuf,
        files: Vec<String>,
        active_file: String,
        editor: Entity<PromptInput>,
        new_file: Option<Entity<PromptInput>>,
        status: Option<Result<String, String>>,
    },
}

fn multiline_input(
    cx: &mut Context<Panel>,
    placeholder: &str,
    initial: &str,
    cancel: impl Fn(&mut App) -> bool + 'static,
) -> Entity<PromptInput> {
    let field = cx.new(|cx| {
        PromptInput::new(cx, placeholder.to_string(), |_, _, _, _| {})
            .with_multiline_code()
            .with_on_overlay_cancel(cancel)
    });
    if !initial.is_empty() {
        field.update(cx, |field, cx| field.set_content(initial.to_string(), cx));
    }
    field
}

fn collect_plugin_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if matches!(name, ".git" | "node_modules" | ".runtime") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file()
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let priority = |rel: &str| match rel {
        "index.mjs" | "index.js" | "index.ts" => 0,
        ".claude-plugin/plugin.json" | "plugin.json" => 1,
        _ if rel.starts_with("commands/") => 2,
        _ if rel.starts_with("skills/") => 3,
        "hooks/hooks.json" | ".mcp.json" => 4,
        "README.md" => 5,
        "jcode.d.ts" => 9,
        _ => 6,
    };
    out.sort_by(|a, b| priority(a).cmp(&priority(b)).then_with(|| a.cmp(b)));
    out
}

fn safe_relative_plugin_path(raw: &str) -> Option<String> {
    let clean = raw.trim().replace('\\', "/");
    let clean = clean.trim_matches('/');
    if clean.is_empty() {
        return None;
    }
    for part in clean.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return None;
        }
    }
    Some(clean.to_owned())
}

const SNIPPET_COMMAND: &str = r#"
  jcode.command("quick-summary", { description: "总结当前项目状态", argumentHint: "[path]" }, (args, ctx) => {
    return `请结合工作目录 ${ctx.cwd} 分析并总结：${args || "整体进度"}`;
  });
"#;

const SNIPPET_TOOL: &str = r#"
  jcode.tool("project_stat", {
    description: "统计给定文本的行数与字符数",
    parameters: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
  }, ({ text }) => JSON.stringify({ lines: String(text).split("\n").length, chars: String(text).length }));
"#;

const SNIPPET_HOOK: &str = r#"
  jcode.on("pre_tool", (event) => {
    if (event.tool === "bash" && /\bgit\s+reset\s+--hard\b/.test(event.input?.command ?? "")) {
      return { block: "已拦截高风险 git reset --hard，请先向用户确认。" };
    }
  });
"#;

const SNIPPET_PANEL: &str = r#"
  jcode.panel("quick-board", {
    title: "插件面板",
    state: { clicks: 0, keyword: "" },
    render: (state, ui) => ui.stack([
      ui.heading("自建插件面板"),
      ui.row([
        ui.input("keyword", { placeholder: "输入备注…" }),
        ui.button("计数 +1", "inc", {}, { variant: "primary" }),
      ], { gap: "sm" }),
      ui.card([ui.keyValue({ "计数": state.clicks, "备注": state.keyword || "-" })], "实时状态"),
    ], { gap: "md", padding: "md" }),
    actions: {
      inc: (_args, state, { toast }) => { toast("已更新", "success"); return { ...state, clicks: (state.clicks || 0) + 1 }; },
    },
  });
"#;

impl Panel {
    /// Open Extensions directly in the AI Plugin Builder view (`/plugin-create`).
    pub(crate) fn open_ai_plugin_builder(
        &mut self,
        existing: Option<(String, PathBuf)>,
        cx: &mut Context<Self>,
    ) {
        if self.extensions.is_none() {
            self.open_extensions(crate::extensions::ExtensionTab::Plugins, cx);
        } else if let Some(state) = self.extensions.as_mut() {
            state.tab = crate::extensions::ExtensionTab::Plugins;
        }
        let weak_name = cx.weak_entity();
        let name = field_input(
            cx,
            tr(
                "optional plugin name (e.g. git-radar — leave blank for AI to choose)",
                "可选插件名（如 git-radar，留空则由 Agent 自动命名）",
            ),
            existing.as_ref().map(|(n, _)| n.as_str()).unwrap_or(""),
            move |app| {
                weak_name
                    .update(app, |panel, cx| panel.extensions_cancel(cx))
                    .unwrap_or(false)
            },
        );
        let weak_req = cx.weak_entity();
        let placeholder = if existing.is_some() {
            tr(
                "Describe what features, commands, hooks, or UI panel changes to make to this plugin…",
                "描述你想给这个插件增加或修改什么功能（如新增斜杠命令、侧栏面板控件、工具拦截规则等）…",
            )
        } else {
            tr(
                "Describe the plugin you want Jcode to build for itself (slash commands, MCP tools, hooks, or a custom sidebar panel)…",
                "描述你想让 Agent 给自己写什么插件（如：做一个统计当前仓库 TODO 并带侧栏筛选面板的插件）…",
            )
        };
        let requirement = multiline_input(cx, placeholder, "", move |app| {
            weak_req
                .update(app, |panel, cx| panel.extensions_cancel(cx))
                .unwrap_or(false)
        });
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        cx.observe(&requirement, |_, _, cx| cx.notify()).detach();
        if let Some(state) = self.extensions.as_mut() {
            state.plugin_form = None;
            state.confirm = None;
            state.scroll.set_offset(point(px(0.), px(0.)));
            state.plugin_studio = Some(PluginStudio::AiBuilder {
                existing,
                name,
                arch: AiPluginArch::Auto,
                requirement,
                error: None,
            });
        }
        cx.notify();
    }

    /// Open the in-app multi-file code editor for a local plugin.
    pub(super) fn open_plugin_code_editor(
        &mut self,
        plugin_name: String,
        plugin_root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let mut files = collect_plugin_files(&plugin_root);
        if files.is_empty() {
            let default_manifest = plugin_root.join(".claude-plugin/plugin.json");
            if let Some(parent) = default_manifest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(
                &default_manifest,
                format!(
                    "{{\n  \"name\": \"{plugin_name}\",\n  \"version\": \"0.1.0\",\n  \"description\": \"Custom Jcode plugin\"\n}}\n"
                ),
            );
            files = collect_plugin_files(&plugin_root);
        }
        let active_file = files
            .first()
            .cloned()
            .unwrap_or_else(|| ".claude-plugin/plugin.json".into());
        let initial = std::fs::read_to_string(plugin_root.join(&active_file)).unwrap_or_default();
        let weak = cx.weak_entity();
        let editor = multiline_input(
            cx,
            tr("Write plugin code here…", "在此编写插件代码…"),
            &initial,
            move |app| {
                weak.update(app, |panel, cx| panel.extensions_cancel(cx))
                    .unwrap_or(false)
            },
        );
        if let Some(state) = self.extensions.as_mut() {
            state.plugin_form = None;
            state.confirm = None;
            state.scroll.set_offset(point(px(0.), px(0.)));
            state.plugin_studio = Some(PluginStudio::CodeEditor {
                plugin_name,
                plugin_root,
                files,
                active_file,
                editor,
                new_file: None,
                status: None,
            });
        }
        cx.notify();
    }

    /// Submit a natural-language plugin creation request directly from `/plugin-create <spec>`.
    pub(super) fn dispatch_ai_plugin_request(
        &mut self,
        existing: Option<(String, PathBuf)>,
        preferred_name: Option<&str>,
        arch: AiPluginArch,
        requirement: &str,
        cx: &mut Context<Self>,
    ) {
        let layout = self
            .extensions
            .as_ref()
            .map(|s| s.plugin_layout())
            .unwrap_or_else(crate::plugins::layout);
        let _ = std::fs::create_dir_all(&layout.plugins_dir);
        let _ = jcode_plugins::creator_skill::ensure(&layout);
        let dispatcher = crate::plugins::dispatcher();
        let existing_ref = existing.as_ref().map(|(n, p)| (n.as_str(), p.as_path()));
        let prompt = jcode_plugins::creator_skill::build_authoring_prompt(
            &layout.plugins_dir,
            dispatcher.as_deref(),
            existing_ref,
            preferred_name,
            arch.hint(),
            requirement.trim(),
        );
        self.close_extensions(cx);
        self.submit_or_queue(prompt, Vec::new(), true, cx);
    }

    pub(super) fn render_plugin_studio(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        let Some(studio) = state.plugin_studio.as_ref() else {
            return div().into_any_element();
        };
        match studio {
            PluginStudio::AiBuilder {
                existing,
                name,
                arch,
                requirement,
                error,
            } => self.render_ai_plugin_builder(
                existing.as_ref(),
                name,
                *arch,
                requirement,
                error.as_deref(),
                &state.scroll,
                cx,
            ),
            PluginStudio::CodeEditor {
                plugin_name,
                plugin_root,
                files,
                active_file,
                editor,
                new_file,
                status,
            } => self.render_plugin_code_editor(
                plugin_name,
                plugin_root,
                files,
                active_file,
                editor,
                new_file.as_ref(),
                status.as_ref(),
                &state.scroll,
                cx,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_ai_plugin_builder(
        &self,
        existing: Option<&(String, PathBuf)>,
        name_field: &Entity<PromptInput>,
        arch: AiPluginArch,
        req_field: &Entity<PromptInput>,
        error: Option<&str>,
        scroll: &ScrollHandle,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let is_edit = existing.is_some();
        let templates: &[(&str, &str, AiPluginArch)] = if is_edit {
            &[
                (
                    tr("+ Add slash command", "+ 新增斜杠命令"),
                    tr(
                        "Add a practical slash command to this plugin with clear argument support and chat output.",
                        "为该插件新增一个实用的斜杠命令，支持命令参数并在聊天中返回结构化结果。",
                    ),
                    AiPluginArch::Auto,
                ),
                (
                    tr("+ Add sidebar panel", "+ 增加侧栏面板"),
                    tr(
                        "Add an interactive sidebar panel (`jcode.panel`) to visualize data and trigger quick actions.",
                        "为该插件增加一个可视化侧栏交互面板（`jcode.panel`），支持筛选输入与快捷操作按钮。",
                    ),
                    AiPluginArch::Code,
                ),
                (
                    tr("+ Add safety hook", "+ 增加安全拦截 Hook"),
                    tr(
                        "Add a `pre_tool` hook to inspect high-risk commands and return `{ block }` with a helpful reason.",
                        "增加 `pre_tool` 钩子检测高危命令并返回 `{ block }` 友好拦截说明。",
                    ),
                    AiPluginArch::Code,
                ),
            ]
        } else {
            &[
                (
                    tr("Git daily report + panel", "Git 提交日报与统计面板"),
                    tr(
                        "Build a code plugin `git-radar` with `/standup [days]` to summarize recent Git commits and a sidebar panel showing branch, uncommitted file counts, and recent commits.",
                        "做一个代码插件 `git-radar`：提供 `/standup [天数]` 命令汇总近期 Git 提交，并在侧栏加一个「Git 看板」面板实时展示当前分支、未提交文件数与最近提交列表。",
                    ),
                    AiPluginArch::Code,
                ),
                (
                    tr("TODO / FIXME scanner", "仓库 TODO 扫描看板"),
                    tr(
                        "Build a code plugin `todo-radar` with a `scan_todos` MCP tool, a `/todos-scan` slash command, and an interactive sidebar panel to filter TODO/FIXME items.",
                        "做一个代码插件 `todo-radar`：提供 `/todos-scan` 命令、供 AI 调用的 `scan_todos` 工具，以及带关键词过滤的 TODO/FIXME 侧栏面板。",
                    ),
                    AiPluginArch::Code,
                ),
                (
                    tr("Dangerous command guard", "高危命令安全护栏"),
                    tr(
                        "Build a safety guard plugin that intercepts destructive `rm -rf`, `git push --force`, and production drop commands in `pre_tool` with clear Chinese explanations.",
                        "做一个安全护栏插件 `safety-shield`：在 `pre_tool` 拦截 `rm -rf`、`git push --force` 等高危操作并给出清晰的中文拦截原因。",
                    ),
                    AiPluginArch::Code,
                ),
                (
                    tr("Code review workflow", "团队代码审查模板命令"),
                    tr(
                        "Build a declarative plugin `team-review` providing `/check-security` and `/check-perf` prompt-template commands plus a code-review skill.",
                        "做一个声明式插件 `team-review`：包含 `/check-security`（安全审计）和 `/check-perf`（性能排查）两个斜杠命令及配套审查技能。",
                    ),
                    AiPluginArch::Declarative,
                ),
            ]
        };

        let arch_pill = |id: &'static str, label: &'static str, value: AiPluginArch, cx: &mut Context<Self>| {
            pill(id, id.into(), label, arch == value).on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(PluginStudio::AiBuilder { arch: slot, .. }) =
                        this.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
                    {
                        *slot = value;
                        cx.notify();
                    }
                }),
            )
        };

        div()
            .id("extensions-plugin-ai-builder")
            .debug_selector(|| "extensions-plugin-ai-builder".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .pr(px(crate::scrollbar::GUTTER))
            .track_scroll(scroll)
            .flex()
            .flex_col()
            .gap_2p5()
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        pill("plugin-ai-back", "plugin-ai-back".into(), tr("Back", "返回"), false)
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.extensions_cancel(cx);
                                }),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(if let Some((name, _)) = existing {
                                if is_zh() {
                                    format!("AI 升级改造插件 · {name}")
                                } else {
                                    format!("AI upgrade plugin · {name}")
                                }
                            } else {
                                tr("AI Self-Plugin Studio", "AI 自扩展插件工坊").to_owned()
                            }),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .px_3()
                    .py_2()
                    .rounded_xl()
                    .bg(theme.HEADER_BG)
                    .text_size(px(11.))
                    .text_color(theme.TEXT_DIM)
                    .child(soft_wrap_text(tr(
                        "Tell Jcode what capability you want — it will write the plugin into `~/.jcode/plugins/`, verify syntax, enable it, and hot-reload slash commands & sidebar panels automatically when finished.",
                        "直接用自然语言描述你想给自己加什么能力——Agent 会自动在 `~/.jcode/plugins/` 下编写插件源码、执行语法校验、自动启用，并在回复完成后无感热加载新命令与侧栏面板！",
                    ))),
            )
            .when(!is_edit, |el| {
                el.child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_none()
                                .w(px(72.))
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(tr("Name", "插件名")),
                        )
                        .child(
                            div()
                                .id("plugin-ai-name")
                                .debug_selector(|| "plugin-ai-name".into())
                                .flex_1()
                                .min_w(px(180.))
                                .child(name_field.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .flex_none()
                                .w(px(72.))
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(tr("Type", "插件形态")),
                        )
                        .child(arch_pill("plugin-ai-arch-auto", tr("Auto (Recommended)", "智能选择（推荐）"), AiPluginArch::Auto, cx))
                        .child(arch_pill("plugin-ai-arch-code", tr("ESM Code + Panel", "ESM 代码 / 面板"), AiPluginArch::Code, cx))
                        .child(arch_pill("plugin-ai-arch-decl", tr("Declarative", "声明式模板"), AiPluginArch::Declarative, cx)),
                )
            })
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_FAINT)
                            .child(tr("Quick templates:", "灵感模板（点击填入）：")),
                    )
                    .children(templates.iter().enumerate().map(|(idx, (title, preset, preset_arch))| {
                        let req_entity = req_field.clone();
                        let preset_text = (*preset).to_owned();
                        let next_arch = *preset_arch;
                        pill(
                            format!("plugin-ai-tpl-{idx}"),
                            format!("plugin-ai-tpl-{idx}"),
                            *title,
                            false,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                req_entity.update(cx, |input, cx| {
                                    input.set_content(preset_text.clone(), cx);
                                });
                                let focus = req_entity.read(cx).focus_handle.clone();
                                focus.focus(window, cx);
                                if let Some(PluginStudio::AiBuilder { arch: slot, error, .. }) =
                                    this.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
                                {
                                    *slot = next_arch;
                                    *error = None;
                                }
                                cx.notify();
                            }),
                        )
                    })),
            )
            .child(
                div()
                    .id("plugin-ai-requirement")
                    .debug_selector(|| "plugin-ai-requirement".into())
                    .w_full()
                    .min_w_0()
                    .child(req_field.clone()),
            )
            .when_some(error.map(str::to_owned), |el, err| {
                el.child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(11.))
                        .text_color(theme.ERROR)
                        .child(soft_wrap_text(&err)),
                )
            })
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        pill(
                            "plugin-ai-submit",
                            "plugin-ai-submit".into(),
                            if is_edit {
                                tr("Ask Agent to upgrade & reload", "交由 Agent 修改并热加载")
                            } else {
                                tr("Ask Agent to build & enable", "交由 Agent 编写并启用")
                            },
                            true,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.submit_ai_plugin_builder(cx);
                            }),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn submit_ai_plugin_builder(&mut self, cx: &mut Context<Self>) {
        let extracted = self
            .extensions
            .as_ref()
            .and_then(|s| s.plugin_studio.as_ref())
            .and_then(|studio| match studio {
                PluginStudio::AiBuilder {
                    existing,
                    name,
                    arch,
                    requirement,
                    ..
                } => Some((
                    existing.clone(),
                    name.read(cx).content.trim().to_owned(),
                    *arch,
                    requirement.read(cx).content.trim().to_owned(),
                )),
                _ => None,
            });
        let Some((existing, preferred_name, arch, req)) = extracted else {
            return;
        };
        if req.is_empty() {
            if let Some(PluginStudio::AiBuilder { error, .. }) =
                self.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
            {
                *error = Some(
                    tr(
                        "Please describe what the plugin should do (or click a quick template above).",
                        "请先输入插件功能描述（或点击上方灵感模板一键填入）。",
                    )
                    .into(),
                );
            }
            cx.notify();
            return;
        }
        let pref = (!preferred_name.is_empty()).then_some(preferred_name.as_str());
        self.dispatch_ai_plugin_request(existing, pref, arch, &req, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn render_plugin_code_editor(
        &self,
        plugin_name: &str,
        plugin_root: &Path,
        files: &[String],
        active_file: &str,
        editor: &Entity<PromptInput>,
        new_file_input: Option<&Entity<PromptInput>>,
        status: Option<&Result<String, String>>,
        scroll: &ScrollHandle,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let is_js = active_file.ends_with(".mjs")
            || active_file.ends_with(".js")
            || active_file.ends_with(".ts");
        let is_cmd_md = active_file.starts_with("commands/") && active_file.ends_with(".md");
        let ask_name = plugin_name.to_owned();
        let ask_root = plugin_root.to_path_buf();

        let snippet_pill = |id: &'static str, label: &'static str, snippet: &'static str, editor: Entity<PromptInput>, cx: &mut Context<Self>| {
            pill(id, id.into(), label, false).on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |_, _, window, cx| {
                    editor.update(cx, |input, cx| {
                        let mut next = input.content.to_string();
                        if let Some(pos) = next.rfind('}') {
                            next.insert_str(pos, snippet);
                        } else {
                            next.push_str(snippet);
                        }
                        input.set_content(next, cx);
                    });
                    let focus = editor.read(cx).focus_handle.clone();
                    focus.focus(window, cx);
                    cx.notify();
                }),
            )
        };

        div()
            .id("extensions-plugin-code-editor")
            .debug_selector(|| "extensions-plugin-code-editor".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .pr(px(crate::scrollbar::GUTTER))
            .track_scroll(scroll)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                pill(
                                    "plugin-editor-back",
                                    "plugin-editor-back".into(),
                                    tr("Back", "返回列表"),
                                    false,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.extensions_cancel(cx);
                                    }),
                                ),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(if is_zh() {
                                        format!("插件工作台 · {plugin_name}")
                                    } else {
                                        format!("Plugin Studio · {plugin_name}")
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(
                                pill(
                                    "plugin-editor-ask-ai",
                                    "plugin-editor-ask-ai".into(),
                                    tr("AI Modify…", "AI 协助修改…"),
                                    false,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        let _ = this.save_plugin_editor_file(false, cx);
                                        this.open_ai_plugin_builder(
                                            Some((ask_name.clone(), ask_root.clone())),
                                            cx,
                                        );
                                    }),
                                ),
                            )
                            .child(
                                pill(
                                    "plugin-editor-save",
                                    "plugin-editor-save".into(),
                                    tr("Save & Reload", "保存并热重载"),
                                    true,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        let _ = this.save_plugin_editor_file(true, cx);
                                    }),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .children(files.iter().enumerate().map(|(idx, rel)| {
                        let rel_owned = rel.clone();
                        let selected = rel == active_file;
                        pill(
                            format!("plugin-editor-file-{idx}"),
                            format!("plugin-editor-file-{idx}"),
                            rel.clone(),
                            selected,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.switch_plugin_editor_file(&rel_owned, window, cx);
                            }),
                        )
                    }))
                    .child(
                        pill(
                            "plugin-editor-new-file",
                            "plugin-editor-new-file".into(),
                            tr("+ File", "+ 新建文件"),
                            new_file_input.is_some(),
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.toggle_plugin_editor_new_file(window, cx);
                            }),
                        ),
                    ),
            )
            .when_some(new_file_input.cloned(), |el, field| {
                el.child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .id("plugin-editor-new-file-input")
                                .debug_selector(|| "plugin-editor-new-file-input".into())
                                .flex_1()
                                .min_w(px(200.))
                                .child(field),
                        )
                        .child(
                            pill(
                                "plugin-editor-new-file-create",
                                "plugin-editor-new-file-create".into(),
                                tr("Create file", "创建文件"),
                                true,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.apply_plugin_editor_new_file(window, cx);
                                }),
                            ),
                        ),
                )
            })
            .when(is_js, |el| {
                el.child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(theme.TEXT_FAINT)
                                .child(tr("Insert snippet:", "快捷插入代码块：")),
                        )
                        .child(snippet_pill("snippet-cmd", tr("+ Command", "+ 斜杠命令"), SNIPPET_COMMAND, editor.clone(), cx))
                        .child(snippet_pill("snippet-tool", tr("+ MCP Tool", "+ AI 工具"), SNIPPET_TOOL, editor.clone(), cx))
                        .child(snippet_pill("snippet-hook", tr("+ Hook", "+ 拦截 Hook"), SNIPPET_HOOK, editor.clone(), cx))
                        .child(snippet_pill("snippet-panel", tr("+ Sidebar Panel", "+ 侧栏面板"), SNIPPET_PANEL, editor.clone(), cx)),
                )
            })
            .when(is_cmd_md, |el| {
                let editor_entity = editor.clone();
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            pill(
                                "snippet-md-cmd",
                                "snippet-md-cmd".into(),
                                tr("+ Frontmatter template", "+ 插入命令模板头"),
                                false,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |_, _, _, cx| {
                                    editor_entity.update(cx, |input, cx| {
                                        if input.content.trim().is_empty() {
                                            input.set_content(
                                                "---\ndescription: 自定义命令说明\nargument-hint: [参数]\n---\n请根据以下要求处理：$ARGUMENTS\n"
                                                    .into(),
                                                cx,
                                            );
                                        }
                                    });
                                    cx.notify();
                                }),
                            ),
                        ),
                )
            })
            .when_some(status.cloned(), |el, res| {
                let (ok, text) = match res {
                    Ok(msg) => (true, msg),
                    Err(err) => (false, err),
                };
                el.child(
                    div()
                        .id("plugin-editor-status")
                        .debug_selector(|| "plugin-editor-status".into())
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .px_3()
                        .py_1p5()
                        .rounded_xl()
                        .bg(theme.HEADER_BG)
                        .text_size(px(11.))
                        .text_color(if ok { theme.OK } else { theme.ERROR })
                        .child(soft_wrap_text(&text)),
                )
            })
            .child(
                div()
                    .id("plugin-editor-body")
                    .debug_selector(|| "plugin-editor-body".into())
                    .w_full()
                    .min_w_0()
                    .child(editor.clone()),
            )
            .into_any_element()
    }

    fn switch_plugin_editor_file(
        &mut self,
        next_rel: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.save_plugin_editor_file(false, cx);
        let Some(PluginStudio::CodeEditor {
            plugin_root,
            active_file,
            editor,
            status,
            ..
        }) = self.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
        else {
            return;
        };
        let content = std::fs::read_to_string(plugin_root.join(next_rel)).unwrap_or_default();
        *active_file = next_rel.to_owned();
        *status = None;
        let editor_clone = editor.clone();
        editor_clone.update(cx, |input, cx| input.set_content(content, cx));
        let focus = editor_clone.read(cx).focus_handle.clone();
        focus.focus(window, cx);
        cx.notify();
    }

    fn toggle_plugin_editor_new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let field = field_input(
            cx,
            tr(
                "relative path (e.g. commands/review.md or index.mjs)",
                "相对路径（如 commands/review.md 或 index.mjs）",
            ),
            "",
            move |app| {
                weak.update(app, |panel, cx| panel.extensions_cancel(cx))
                    .unwrap_or(false)
            },
        );
        if let Some(PluginStudio::CodeEditor { new_file, .. }) =
            self.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
        {
            if new_file.is_some() {
                *new_file = None;
            } else {
                let focus = field.read(cx).focus_handle.clone();
                *new_file = Some(field);
                focus.focus(window, cx);
            }
        }
        cx.notify();
    }

    fn apply_plugin_editor_new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self
            .extensions
            .as_ref()
            .and_then(|s| s.plugin_studio.as_ref())
            .and_then(|studio| match studio {
                PluginStudio::CodeEditor {
                    new_file: Some(input),
                    ..
                } => Some(input.read(cx).content.to_string()),
                _ => None,
            })
            .unwrap_or_default();
        let Some(rel) = safe_relative_plugin_path(&raw) else {
            if let Some(PluginStudio::CodeEditor { status, .. }) =
                self.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
            {
                *status = Some(Err(tr(
                    "Enter a valid relative file path inside the plugin folder.",
                    "请输入合法的插件内相对文件路径（例如 commands/my-cmd.md）。",
                )
                .into()));
            }
            cx.notify();
            return;
        };
        let created = if let Some(PluginStudio::CodeEditor {
            plugin_root,
            files,
            new_file,
            ..
        }) = self.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
        {
            let target = plugin_root.join(&rel);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if !target.exists() {
                let starter = if rel.ends_with(".mjs") || rel.ends_with(".js") {
                    "/** @param {import(\"./jcode\").Jcode} jcode */\nexport default function (jcode) {\n}\n"
                } else if rel.starts_with("commands/") && rel.ends_with(".md") {
                    "---\ndescription: 自定义命令\nargument-hint: [参数]\n---\n$ARGUMENTS\n"
                } else if rel.ends_with(".json") {
                    "{\n}\n"
                } else {
                    ""
                };
                let _ = std::fs::write(&target, starter);
            }
            *files = collect_plugin_files(plugin_root);
            *new_file = None;
            true
        } else {
            false
        };
        if created {
            self.switch_plugin_editor_file(&rel, window, cx);
        }
    }

    pub(super) fn save_plugin_editor_file(
        &mut self,
        reload_and_validate: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let info = self
            .extensions
            .as_ref()
            .and_then(|s| {
                let layout = s.plugin_layout();
                match s.plugin_studio.as_ref()? {
                    PluginStudio::CodeEditor {
                        plugin_name,
                        plugin_root,
                        active_file,
                        editor,
                        ..
                    } => Some((
                        layout,
                        plugin_name.clone(),
                        plugin_root.clone(),
                        active_file.clone(),
                        editor.read(cx).content.to_string(),
                    )),
                    _ => None,
                }
            });
        let Some((layout, plugin_name, plugin_root, active_file, content)) = info else {
            return false;
        };
        let target = plugin_root.join(&active_file);
        if let Err(err) = jcode_plugins::sync::write_atomic(&target, content.as_bytes()) {
            if let Some(PluginStudio::CodeEditor { status, .. }) =
                self.extensions.as_mut().and_then(|s| s.plugin_studio.as_mut())
            {
                *status = Some(Err(if is_zh() {
                    format!("保存 `{active_file}` 失败：{err}")
                } else {
                    format!("Could not save `{active_file}`: {err}")
                }));
            }
            cx.notify();
            return false;
        }
        if !reload_and_validate {
            return true;
        }
        let report = crate::plugins::sync_in(&layout);
        if layout == crate::plugins::layout() {
            crate::applet_runtime::get(cx).reload_plugin_applets();
        }
        let outcome = if let Some(warn) = report
            .warnings
            .iter()
            .find(|w| w.contains(&plugin_name))
        {
            Err(if is_zh() {
                format!("已保存 `{active_file}`，但插件校验报错：{warn}")
            } else {
                format!("Saved `{active_file}`, but validation failed: {warn}")
            })
        } else {
            let (discovered, errors) = jcode_plugins::discover(&layout, &jcode_plugins::State::load(&layout));
            if let Some(err) = errors.iter().find(|e| e.contains(&plugin_name)) {
                Err(if is_zh() {
                    format!("清单解析错误：{err}")
                } else {
                    format!("Manifest error: {err}")
                })
            } else if let Some(p) = discovered.iter().find(|p| p.name == plugin_name) {
                let row = super::extensions::PluginRow::from_plugin(p);
                Ok(if is_zh() {
                    format!(
                        "✓ 已保存 `{active_file}` 并热重载生效（命令 {} · 技能 {} · Hook {} · MCP {} · 面板 {}）",
                        row.commands.len(),
                        row.skills,
                        row.hook_events,
                        row.mcp_servers,
                        row.applets
                    )
                } else {
                    format!(
                        "✓ Saved `{active_file}` and hot-reloaded ({} commands · {} skills · {} hooks · {} MCP · {} panels)",
                        row.commands.len(),
                        row.skills,
                        row.hook_events,
                        row.mcp_servers,
                        row.applets
                    )
                })
            } else {
                Ok(if is_zh() {
                    format!("✓ 已保存 `{active_file}` 并重新同步。")
                } else {
                    format!("✓ Saved `{active_file}` and synced.")
                })
            }
        };
        if let Some(state) = self.extensions.as_mut() {
            state.reload();
            if let Some(PluginStudio::CodeEditor { files, status, plugin_root, .. }) =
                state.plugin_studio.as_mut()
            {
                *files = collect_plugin_files(plugin_root);
                *status = Some(outcome);
            }
        }
        self.plugins_fingerprint = crate::plugins::plugins_fingerprint();
        cx.notify();
        true
    }
}
