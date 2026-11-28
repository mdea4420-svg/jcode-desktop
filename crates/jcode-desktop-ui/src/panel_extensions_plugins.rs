//! Extensions overlay — plugin browsing and management.

use std::collections::BTreeSet;
use std::path::Path;

use super::*;
use super::extensions::{Confirm, PluginFilter, PluginForm, PluginRow, field_input, pill, soft_wrap_text};
use super::extensions_skills::percent_encode_path;
use crate::locale::{is_zh, tr};

impl PluginRow {
    pub(super) fn from_plugin(plugin: &jcode_plugins::Plugin) -> Self {
        let code = plugin.code.as_ref();
        let commands = jcode_plugins::commands::load(plugin)
            .into_iter()
            .map(|command| format!("/{}", command.qualified()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let (hooks, _) = jcode_plugins::hooks::plugin_hooks(plugin).unwrap_or_default();
        let mut events: BTreeSet<_> = hooks.iter().map(|(event, _)| *event).collect();
        if let Some(code) = code {
            events.extend(code.events.iter().map(String::as_str));
        }
        let applets = plugin.applet_dirs().iter().map(|dir| {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().join("applet.json").is_file())
                .count()
        }).sum::<usize>();
        Self {
            name: plugin.name.clone(),
            version: plugin.version.clone().unwrap_or_default(),
            code: plugin.main_script().is_some() || code.is_some(),
            source: plugin.source.label().into(),
            enabled: plugin.enabled,
            root: plugin.root.clone(),
            description: plugin.description.clone(),
            commands,
            skills: plugin.skills().len(),
            hook_events: events.len(),
            mcp_servers: plugin.mcp_servers().map_or(0, |servers| servers.len())
                + usize::from(code.is_some_and(|code| !code.tools.is_empty())),
            applets: applets + code.map_or(0, |code| code.panels.len()),
        }
    }

    fn matches(&self, query: &str) -> bool {
        let types = if self.code { ["code", "代码"] } else { ["declarative", "声明式"] };
        [self.name.as_str(), self.source.as_str(), types[0], types[1]]
            .into_iter()
            .chain(self.description.as_deref())
            .chain(self.commands.iter().map(String::as_str))
            .any(|text| text.to_lowercase().contains(query))
    }

    fn matches_filter(&self, filter: PluginFilter) -> bool {
        match filter {
            PluginFilter::All => true,
            PluginFilter::Enabled => self.enabled,
            PluginFilter::Local => self.source != "claude",
            PluginFilter::Claude => self.source == "claude",
        }
    }
}

fn open_folder(path: &Path, cx: &mut App) {
    cx.open_url(&format!("file://{}", percent_encode_path(path)));
}

fn plugin_summary(report: &jcode_plugins::SyncReport) -> String {
    crate::plugins::report_markdown(report).lines().next().unwrap_or_default().into()
}

impl Panel {
    pub(super) fn render_extensions_plugins(&self, query: &str, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        if let Some(form) = &state.plugin_form {
            return self.render_plugin_form(form, cx);
        }

        let active_filter = state.plugin_filter;
        let total_count = state.plugins.len();
        let enabled_count = state.plugins.iter().filter(|p| p.enabled).count();
        let local_count = state.plugins.iter().filter(|p| p.source != "claude").count();
        let claude_count = state.plugins.iter().filter(|p| p.source == "claude").count();
        let plugins_dir = state.plugin_layout().plugins_dir;
        let plugin_errors = state.plugin_errors.clone();

        let matches: Vec<_> = state
            .plugins
            .iter()
            .filter(|plugin| plugin.matches_filter(active_filter) && plugin.matches(query))
            .collect();

        let filter_pill = |filter: PluginFilter, slug: &'static str, label: String, cx: &mut Context<Self>| {
            let id = format!("plugins-filter-{slug}");
            pill(id.clone(), id, label, active_filter == filter).on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(state) = this.extensions.as_mut() {
                        state.plugin_filter = filter;
                        state.confirm = None;
                        state.scroll.set_offset(point(px(0.), px(0.)));
                    }
                    cx.notify();
                }),
            )
        };

        let toolbar = div()
            .flex_none()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .child(
                pill("plugins-new", "plugins-new".into(), tr("+ New plugin", "+ 新建插件"), true)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| {
                        this.open_plugin_form(window, cx);
                    })),
            )
            .child(
                pill("plugins-open-dir", "plugins-open-dir".into(), tr("Open plugins folder", "打开插件目录"), false)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        if let Some(state) = &this.extensions {
                            let path = state.plugin_layout().plugins_dir;
                            match std::fs::create_dir_all(&path) {
                                Ok(()) => open_folder(&path, cx),
                                Err(error) => this.extensions_apply(Err(if is_zh() {
                                    format!("无法打开插件目录：{error}")
                                } else {
                                    format!("Could not open the plugins folder: {error}")
                                }), "", cx),
                            }
                        }
                    })),
            )
            .child(
                pill("plugins-reload", "plugins-reload".into(), tr("Reload", "重新加载"), false)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        if let Some(state) = &this.extensions {
                            let report = crate::plugins::sync_in(&state.plugin_layout());
                            this.extensions_apply(Ok(plugin_summary(&report)), tr("Reloaded:", "已重新加载："), cx);
                        }
                    })),
            )
            .when(total_count > 0, |el| {
                el.child(div().flex_1().min_w(px(8.)))
                    .child(filter_pill(
                        PluginFilter::All,
                        "all",
                        format!("{} {total_count}", tr("All", "全部")),
                        cx,
                    ))
                    .child(filter_pill(
                        PluginFilter::Enabled,
                        "enabled",
                        format!("{} {enabled_count}", tr("Enabled", "已启用")),
                        cx,
                    ))
                    .when(local_count > 0 && claude_count > 0, |el| {
                        el.child(filter_pill(
                            PluginFilter::Local,
                            "local",
                            format!("{} {local_count}", tr("Local", "本地")),
                            cx,
                        ))
                        .child(filter_pill(
                            PluginFilter::Claude,
                            "claude",
                            format!("Claude {claude_count}"),
                            cx,
                        ))
                    })
            });

        div()
            .id("extensions-plugins-list")
            .debug_selector(|| "extensions-plugins-list".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1p5()
            .overflow_hidden()
            .pr(px(crate::scrollbar::GUTTER))
            .track_scroll(&state.scroll)
            .child(toolbar)
            .children(plugin_errors.into_iter().map(|err| {
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
                    .text_color(theme.WARN)
                    .child(soft_wrap_text(&format!("{}{err}", tr("Plugin load warning: ", "插件加载提示："))))
            }))
            .when(matches.is_empty(), |el| {
                el.child(
                    div()
                        .id("extensions-plugins-empty")
                        .debug_selector(|| "extensions-plugins-empty".into())
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .p_3()
                        .rounded_xl()
                        .bg(theme.HEADER_BG)
                        .text_size(px(12.))
                        .text_color(theme.TEXT_DIM)
                        .child(if state.plugins.is_empty() {
                            tr("No plugins installed. Click \"+ New plugin\" above to scaffold one, or open the plugins folder.", "尚未安装插件。可点击上方「+ 新建插件」创建脚手架，或打开插件目录放入已有插件。")
                        } else {
                            tr("Nothing matches the filter.", "没有匹配的插件。")
                        }),
                )
            })
            .children(matches.into_iter().map(|plugin| self.render_plugin_row(plugin, &plugins_dir, cx)))
            .into_any_element()
    }

    fn render_plugin_row(
        &self,
        plugin: &PluginRow,
        plugins_dir: &Path,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let name = plugin.name.clone();
        let toggle_name = name.clone();
        let delete_name = name.clone();
        let confirm_name = name.clone();
        let open_path = plugin.root.clone();
        let confirm_root = plugin.root.clone();
        let removable = plugin.source != "claude" && plugin.root.starts_with(plugins_dir);
        let confirming_delete = self.extensions.as_ref().is_some_and(|state| {
            matches!(&state.confirm, Some(Confirm::PluginDelete { name: n }) if n == &name)
        });
        let tooltip = plugin.root.display().to_string();
        let compact_root = crate::platform_dirs::tilde(&tooltip);
        let enabled = plugin.enabled;
        let kind = if plugin.code { tr("ESM code", "代码插件") } else { tr("declarative", "声明式") };
        let source = if plugin.source == "claude" { tr("claude", "Claude Code") } else { tr("local", "本地插件") };

        let meta_chip = |label: String, accent: bool| {
            div()
                .flex_none()
                .px_2()
                .py(px(1.))
                .rounded_full()
                .bg(if accent { theme.ACCENT_DIM } else { theme.TOOL_BG })
                .text_size(px(10.))
                .text_color(if accent { theme.ACCENT } else { theme.TEXT_DIM })
                .whitespace_nowrap()
                .child(label)
        };

        let caps: Vec<String> = plugin
            .commands
            .iter()
            .cloned()
            .chain(
                [
                    (plugin.skills, tr("skills", "技能")),
                    (plugin.hook_events, tr("hooks", "Hook")),
                    (plugin.mcp_servers, tr("MCP", "MCP")),
                    (plugin.applets, tr("panels", "面板")),
                ]
                .into_iter()
                .filter(|(count, _)| *count > 0)
                .map(|(count, label)| format!("{count} {label}")),
            )
            .collect();

        let badges = caps.iter().map(|label| {
            div()
                .flex_none()
                .max_w(px(240.))
                .min_w_0()
                .overflow_hidden()
                .truncate()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(theme.TOOL_BG)
                .text_size(px(10.))
                .text_color(theme.TEXT_DIM)
                .child(label.clone())
        });

        div()
            .id(format!("plugin-row-{}", name.clone()))
            .debug_selector({
                let name = name.clone();
                move || format!("plugin-row-{name}").into()
            })
            .flex_none()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap_1p5()
            .px_3()
            .py_2p5()
            .rounded_xl()
            .bg(theme.HEADER_BG)
            .tooltip(move |_, cx| cx.new(|_| super::usage::MeterTooltip(tooltip.clone())).into())
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .overflow_hidden()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .max_w(px(240.))
                                    .min_w_0()
                                    .overflow_hidden()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .when(!enabled, |el| el.text_color(theme.TEXT_DIM))
                                    .child(name.clone()),
                            )
                            .child(meta_chip(kind.to_owned(), plugin.code))
                            .child(meta_chip(source.to_owned(), false))
                            .when(!plugin.version.is_empty(), |el| {
                                el.child(meta_chip(format!("v{}", plugin.version.trim_start_matches('v')), false))
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(
                                pill(
                                    format!("plugin-open-{name}"),
                                    format!("plugin-open-{name}"),
                                    tr("Open folder", "打开目录"),
                                    false,
                                )
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    open_folder(&open_path, cx)
                                }),
                            )
                            .when(removable, |el| {
                                el.child(
                                    pill(
                                        format!("plugin-delete-{}", delete_name.clone()),
                                        format!("plugin-delete-{delete_name}"),
                                        tr("Remove", "移除"),
                                        false,
                                    )
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            if let Some(state) = this.extensions.as_mut() {
                                                state.confirm = Some(Confirm::PluginDelete {
                                                    name: delete_name.clone(),
                                                });
                                            }
                                            cx.notify();
                                        }),
                                    ),
                                )
                            })
                            .child(
                                pill(
                                    format!("plugin-toggle-{}", toggle_name.clone()),
                                    format!("plugin-toggle-{toggle_name}"),
                                    if enabled {
                                        tr("Enabled", "已启用")
                                    } else {
                                        tr("Disabled", "已禁用")
                                    },
                                    enabled,
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        if let Some(state) = &this.extensions {
                                            let result = crate::plugins::set_enabled_in(
                                                &state.plugin_layout(),
                                                &toggle_name,
                                                !enabled,
                                            )
                                            .map(|report| plugin_summary(&report));
                                            let done = if enabled {
                                                tr("Disabled:", "已禁用：")
                                            } else {
                                                tr("Enabled:", "已启用：")
                                            };
                                            this.extensions_apply(result, done, cx);
                                        }
                                    }),
                                ),
                            ),
                    ),
            )
            .when_some(
                plugin.description.clone().filter(|text| !text.trim().is_empty()),
                |el, description| {
                    el.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .child(soft_wrap_text(description.trim())),
                    )
                },
            )
            .when(!caps.is_empty(), |el| {
                el.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(badges),
                )
            })
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_size(px(10.))
                    .font_family(theme.FONT_MONO)
                    .text_color(theme.TEXT_FAINT)
                    .child(compact_root),
            )
            .when(confirming_delete, |el| {
                let confirm_del_id = confirm_name.clone();
                let confirm_cancel_id = confirm_name.clone();
                el.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .pt_1()
                        .debug_selector({
                            let confirm_dbg = confirm_name.clone();
                            move || format!("plugin-confirm-{confirm_dbg}").into()
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(160.))
                                .overflow_hidden()
                                .text_size(px(11.))
                                .text_color(theme.TEXT_DIM)
                                .child(soft_wrap_text(&if is_zh() {
                                    format!("将插件 `{confirm_name}` 移入 `~/.jcode/trash/plugins` 回收站？")
                                } else {
                                    format!("Move plugin `{confirm_name}` to `~/.jcode/trash/plugins`?")
                                })),
                        )
                        .child(
                            pill(
                                format!("plugin-confirm-delete-{confirm_del_id}"),
                                format!("plugin-confirm-delete-{confirm_del_id}"),
                                tr("Move to trash", "移入回收站"),
                                true,
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    let layout = this.extensions.as_ref().map(|s| s.plugin_layout());
                                    if let Some(state) = this.extensions.as_mut() {
                                        state.confirm = None;
                                    }
                                    if let Some(layout) = layout {
                                        let result = crate::plugins::trash_in(
                                            &layout,
                                            &confirm_name,
                                            &confirm_root,
                                        )
                                        .map(|dest| format!("`{confirm_name}` → `{}`.", dest.display()));
                                        this.extensions_apply(
                                            result,
                                            tr("Moved to trash:", "已移入回收站："),
                                            cx,
                                        );
                                    }
                                }),
                            ),
                        )
                        .child(
                            pill(
                                format!("plugin-confirm-cancel-{confirm_cancel_id}"),
                                format!("plugin-confirm-cancel-{confirm_cancel_id}"),
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
            .into_any_element()
    }

    fn open_plugin_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let name = field_input(cx, tr("name (kebab-case, e.g. my-helper)", "插件名称（连字符格式，如 my-helper）"), "", move |app| {
            weak.update(app, |panel, cx| panel.extensions_cancel(cx)).unwrap_or(false)
        });
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        if let Some(state) = self.extensions.as_mut() {
            let focus = name.read(cx).focus_handle.clone();
            state.plugin_form = Some(PluginForm { name, code: false, error: None });
            state.scroll.set_offset(point(px(0.), px(0.)));
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn render_plugin_form(&self, form: &PluginForm, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        let sanitized = jcode_plugins::manifest::sanitize_name(form.name.read(cx).content.trim());
        let preview_name = if sanitized.is_empty() { "<name>" } else { sanitized.as_str() };
        let preview_dir = crate::platform_dirs::tilde(
            &state
                .plugin_layout()
                .plugins_dir
                .join(preview_name)
                .display()
                .to_string(),
        );
        let label = |label: &'static str| {
            div().flex_none().w(px(80.)).text_size(px(11.)).text_color(theme.TEXT_DIM).child(label)
        };
        let type_pill = |id: &'static str, text: &'static str, code: bool| {
            pill(id, id.into(), text, form.code == code)
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this.extensions.as_mut().and_then(|state| state.plugin_form.as_mut()) {
                        form.code = code;
                        cx.notify();
                    }
                }))
        };
        let kind_desc = if form.code {
            tr(
                "Generates plugin.json + index.mjs + jcode.d.ts. Supports dynamic Node ESM slash commands, tools, lifecycle hooks, and custom sidebar panels.",
                "生成 plugin.json + index.mjs + jcode.d.ts。支持单文件 Node ESM 编写动态斜杠命令、工具、生命周期 Hook 与自定义侧栏面板。",
            )
        } else {
            tr(
                "Generates plugin.json + commands/hello.md. Ideal for prompt-template slash commands, skills, .mcp.json servers, and declarative hooks.",
                "生成 plugin.json + commands/hello.md。适合快速封装提示词模板命令、技能目录、.mcp.json 服务与声明式 Hook。",
            )
        };
        div()
            .id("extensions-plugin-form")
            .debug_selector(|| "extensions-plugin-form".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .pr(px(crate::scrollbar::GUTTER))
            .track_scroll(&state.scroll)
            .flex()
            .flex_col()
            .gap_2p5()
            .child(div().flex_none().flex().items_center().gap_2()
                .child(pill("plugins-form-back", "plugins-form-back".into(), tr("Back", "返回"), false)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        this.extensions_cancel(cx);
                    })))
                .child(div().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(tr("New plugin", "新建插件"))))
            .child(div().flex_none().w_full().min_w_0().flex().items_center().gap_2()
                .child(label(tr("name", "名称")))
                .child(div().id("plugins-form-name").debug_selector(|| "plugins-form-name".into())
                    .flex_1().min_w_0().child(form.name.clone())))
            .child(div().flex_none().w_full().min_w_0().flex().flex_wrap().items_center().gap_2()
                .child(label(tr("type", "类型")))
                .child(type_pill("plugins-kind-declarative", tr("Declarative", "声明式"), false))
                .child(type_pill("plugins-kind-code", tr("ESM code", "ESM 代码"), true)))
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
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .child(kind_desc),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .truncate()
                            .text_size(px(10.))
                            .font_family(theme.FONT_MONO)
                            .text_color(theme.TEXT_FAINT)
                            .child(format!("{}{preview_dir}", tr("Target folder: ", "目标目录："))),
                    ),
            )
            .when_some(form.error.clone(), |el, error| {
                el.child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(theme.ERROR)
                        .text_size(px(11.))
                        .child(soft_wrap_text(&error)),
                )
            })
            .child(div().flex_none().child(pill("plugins-create", "plugins-create".into(), tr("Create plugin", "创建插件"), true)
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| this.save_plugin_form(cx)))))
            .into_any_element()
    }

    fn save_plugin_form(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &self.extensions else { return; };
        let Some(form) = &state.plugin_form else { return; };
        let name = form.name.read(cx).content.trim();
        let result = if jcode_plugins::manifest::sanitize_name(name).starts_with('.') {
            Err(tr("Plugin names cannot start with a dot.", "插件名称不能以点开头。").to_owned())
        } else {
            crate::plugins::create_in(&state.plugin_layout(), name, form.code)
        };
        if let Some(state) = self.extensions.as_mut() {
            match result {
                Ok(root) => {
                    state.plugin_form = None;
                    state.focus_pending = true;
                    state.scroll.set_offset(point(px(0.), px(0.)));
                    self.extensions_apply(Ok(root.display().to_string()), tr("Created", "已创建"), cx);
                }
                Err(error) => {
                    if let Some(form) = &mut state.plugin_form {
                        form.error = Some(error);
                        cx.notify();
                    }
                }
            }
        }
    }
}
