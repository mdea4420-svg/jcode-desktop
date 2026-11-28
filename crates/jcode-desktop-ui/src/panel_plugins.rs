//! `/plugins` and plugin slash commands in the chat panel.
use super::*;
use crate::locale::{is_zh, tr};

impl Panel {
    /// `/plugins [enable|disable|new|reload|ai] [name]` and `/plugin-create <spec>`.
    pub(super) fn handle_plugins_command(&mut self, content: &str, cx: &mut Context<Self>) -> bool {
        if let Some(spec) = content.strip_prefix("/plugin-create").map(str::trim) {
            if spec.is_empty() {
                self.open_ai_plugin_builder(None, cx);
            } else {
                self.dispatch_ai_plugin_request(
                    None,
                    None,
                    super::plugin_studio::AiPluginArch::Auto,
                    spec,
                    cx,
                );
            }
            return true;
        }
        let mut words = content.split_whitespace();
        let Some(cmd) = words.next() else {
            return false;
        };
        if !matches!(cmd, "/plugins" | "/plugin") {
            return false;
        }
        let rest = content[cmd.len()..].trim();
        if let Some(spec) = rest.strip_prefix("ai").map(str::trim) {
            if spec.is_empty() {
                self.open_ai_plugin_builder(None, cx);
            } else {
                self.dispatch_ai_plugin_request(
                    None,
                    None,
                    super::plugin_studio::AiPluginArch::Auto,
                    spec,
                    cx,
                );
            }
            return true;
        }
        let action = words.next().unwrap_or("list");
        let name = words.next();
        let code = words.next().is_some_and(|flag| matches!(flag, "code" | "--code"));
        let item = match (action, name) {
            ("list" | "status", _) => Item::Assistant(crate::plugins::summary_markdown()),
            ("enable" | "disable", Some(name)) => {
                match crate::plugins::set_enabled(name, action == "enable") {
                    Ok(report) => {
                        let summary = if is_zh() {
                            let status = if action == "enable" { "开启" } else { "关闭" };
                            format!("已{status}插件 `{name}`。")
                        } else {
                            format!("Plugin `{name}` {action}d.")
                        };
                        Item::Assistant(format!("{summary}\n\n{}", crate::plugins::report_markdown(&report)))
                    }
                    Err(error) => Item::Error(error),
                }
            }
            ("new", Some(name)) => match crate::plugins::create(name, code) {
                Ok(root) => Item::Assistant(if code {
                    let path = root.display();
                    if is_zh() {
                        format!("已在 `{path}` 创建代码插件。编辑 `index.mjs` 后运行 `/plugins reload`。可试用 `/standup`，重启桌面端后可从侧栏打开插件面板。")
                    } else {
                        format!("Created a code plugin at `{path}`. Edit `index.mjs`, then run `/plugins reload`. Try `/standup`, and open its panel from the sidebar after restarting Desktop.")
                    }
                } else {
                    let path = root.display();
                    if is_zh() {
                        format!("已在 `{path}` 创建示例插件。可先试用 `/hello`。编辑插件文件后，运行 `/plugins reload` 加载更改。")
                    } else {
                        format!("Created a starter plugin at `{path}`. Try `/hello`, then edit its files and run `/plugins reload`.")
                    }
                }),
                Err(error) => Item::Error(error),
            },
            ("reload" | "sync", _) => {
                Item::Assistant(crate::plugins::report_markdown(&crate::plugins::sync_and_hot_reload(cx)))
            }
            _ => Item::Error(
                tr(
                    "Usage: `/plugins`, `/plugin-create <spec>`, `/plugins enable <name>`, `/plugins disable <name>`, `/plugins new <name>`, `/plugins reload`.",
                    "用法：`/plugins` 打开插件面板，`/plugin-create <需求描述>` 让 Agent 自动编写并安装插件，`/plugins enable|disable <name>` 开关，`/plugins reload` 热重载。",
                ).into(),
            ),
        };
        self.items.push(item);
        cx.notify();
        true
    }

    /// Send a template command's prompt, or run a code command in the
    /// background and then send its prompt or show its message.
    pub(super) fn handle_plugin_slash_command(&mut self, content: &str, cx: &mut Context<Self>) -> bool {
        let Some(invocation) = crate::plugins::invocation(content) else {
            return false;
        };
        let (plugin, name, args) = match invocation {
            crate::plugins::Invocation::Prompt(prompt) => {
                self.submit_or_queue(prompt, Vec::new(), true, cx);
                return true;
            }
            crate::plugins::Invocation::Code { plugin, name, args } => (plugin, name, args),
        };
        let cwd = self
            .working_dir
            .clone()
            .filter(|dir| !dir.trim().is_empty())
            .map(std::path::PathBuf::from);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::plugins::run_code_command(&plugin, &name, &args, cwd.as_deref())
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                match result {
                    Ok(reply) => {
                        if let Some(error) = reply.error {
                            panel.items.push(Item::Error(error));
                        } else if let Some(prompt) = reply.prompt.filter(|p| !p.trim().is_empty()) {
                            panel.submit_or_queue(prompt, Vec::new(), true, cx);
                        } else if let Some(message) = reply.message {
                            panel.items.push(Item::Assistant(message));
                        }
                    }
                    Err(error) => panel.items.push(Item::Error(error)),
                }
                panel.transcript_measurements.dirty = true;
                cx.notify();
            });
        })
        .detach();
        true
    }
}
