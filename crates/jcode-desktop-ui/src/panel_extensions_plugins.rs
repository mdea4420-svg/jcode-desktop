//! Extensions overlay — plugin browsing and management.

use std::collections::BTreeSet;
use std::path::Path;

use super::*;
use super::extensions::{PluginForm, PluginRow, field_input, pill};
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

        let matches: Vec<_> = state.plugins.iter().filter(|plugin| plugin.matches(query)).collect();
        let toolbar = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                pill("plugins-new", "plugins-new".into(), tr("+ New plugin", "+ 新建插件"), false)
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
            );
        div()
            .id("extensions-plugins-list")
            .debug_selector(|| "extensions-plugins-list".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .overflow_y_scroll()
            .track_scroll(&state.scroll)
            .child(toolbar)
            .when(matches.is_empty(), |el| {
                el.child(
                    div()
                        .id("extensions-plugins-empty")
                        .debug_selector(|| "extensions-plugins-empty".into())
                        .flex_none()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(theme.TEXT_DIM)
                        .child(if state.plugins.is_empty() {
                            tr("No plugins installed. Run `/plugins` for details.", "尚未安装插件。可在上方新建插件，或运行 `/plugins` 查看详情。")
                        } else {
                            tr("Nothing matches the filter.", "没有匹配的插件。")
                        }),
                )
            })
            .children(matches.into_iter().map(|plugin| self.render_plugin_row(plugin, cx)))
            .into_any_element()
    }

    fn render_plugin_row(&self, plugin: &PluginRow, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let name = plugin.name.clone();
        let toggle_name = name.clone();
        let open_path = plugin.root.clone();
        let tooltip = plugin.root.display().to_string();
        let enabled = plugin.enabled;
        let kind = if plugin.code { tr("code", "代码") } else { tr("declarative", "声明式") };
        let source = if plugin.source == "claude" { tr("claude", "Claude Code") } else { &plugin.source };
        let mut detail = format!("{kind} · {source}");
        if !plugin.version.is_empty() {
            detail.push_str(&format!(" · {}", plugin.version));
        }
        let caps = plugin.commands.iter().cloned().chain([
            (plugin.skills, tr("skills", "技能")),
            (plugin.hook_events, tr("hook events", "Hook 事件")),
            (plugin.mcp_servers, tr("MCP servers", "MCP 服务器")),
            (plugin.applets, tr("panels", "面板")),
        ].into_iter().filter(|(count, _)| *count > 0).map(|(count, label)| format!("{count} {label}")));
        let badges = caps.map(|label| {
            div().px_2().py_0p5().rounded_full().bg(theme.TOOL_BG)
                .text_size(px(10.)).text_color(theme.TEXT_DIM).child(label)
        });
        div()
            .id(format!("plugin-row-{}", name.clone()))
            .debug_selector({
                let name = name.clone();
                move || format!("plugin-row-{name}").into()
            })
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_xl()
            .bg(theme.HEADER_BG)
            .tooltip(move |_, cx| cx.new(|_| super::usage::MeterTooltip(tooltip.clone())).into())
            .child(
                div().flex().flex_wrap().items_center().gap_2()
                    .child(div().max_w(px(200.)).truncate().text_size(px(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .when(!enabled, |el| el.text_color(theme.TEXT_DIM)).child(name.clone()))
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(theme.TEXT_DIM).child(detail))
                    .child(pill(format!("plugin-open-{name}"), format!("plugin-open-{name}"), tr("Open folder", "打开目录"), false)
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| open_folder(&open_path, cx)))
                    .child(pill(
                        format!("plugin-toggle-{}", toggle_name.clone()),
                        format!("plugin-toggle-{toggle_name}"),
                        if enabled { tr("On", "开启") } else { tr("Off", "关闭") },
                        enabled,
                    ).on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        if let Some(state) = &this.extensions {
                            let result = crate::plugins::set_enabled_in(&state.plugin_layout(), &toggle_name, !enabled)
                                .map(|report| plugin_summary(&report));
                            let done = if enabled { tr("Disabled:", "已关闭：") } else { tr("Enabled:", "已开启：") };
                            this.extensions_apply(result, done, cx);
                        }
                    }))),
            )
            .when_some(plugin.description.clone().filter(|text| !text.is_empty()), |el, description| {
                el.child(div().text_size(px(11.)).text_color(theme.TEXT_DIM).child(description))
            })
            .child(div().flex().flex_wrap().gap_1().children(badges))
            .into_any_element()
    }

    fn open_plugin_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let name = field_input(cx, tr("name (kebab-case)", "名称（用连字符分隔）"), "", move |app| {
            weak.update(app, |panel, cx| panel.extensions_cancel(cx)).unwrap_or(false)
        });
        if let Some(state) = self.extensions.as_mut() {
            let focus = name.read(cx).focus_handle.clone();
            state.plugin_form = Some(PluginForm { name, code: false, error: None });
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn render_plugin_form(&self, form: &PluginForm, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
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
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().items_center().gap_2()
                .child(pill("plugins-form-back", "plugins-form-back".into(), tr("Back", "返回"), false)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        this.extensions_cancel(cx);
                    })))
                .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::MEDIUM)
                    .child(tr("New plugin", "新建插件"))))
            .child(div().flex().items_center().gap_2()
                .child(label(tr("name", "名称")))
                .child(div().id("plugins-form-name").debug_selector(|| "plugins-form-name".into())
                    .flex_1().min_w_0().child(form.name.clone())))
            .child(div().flex().items_center().gap_2()
                .child(label(tr("type", "类型")))
                .child(type_pill("plugins-kind-declarative", tr("Declarative", "声明式"), false))
                .child(type_pill("plugins-kind-code", tr("ESM code", "ESM 代码"), true)))
            .when_some(form.error.clone(), |el, error| {
                el.child(div().text_color(theme.ERROR).text_size(px(11.)).child(error))
            })
            .child(div().child(pill("plugins-create", "plugins-create".into(), tr("Create", "创建"), true)
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
