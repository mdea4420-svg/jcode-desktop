//! `/plugins` and plugin slash commands in the chat panel.
use super::*;

impl Panel {
    /// `/plugins [enable|disable|new|reload] [name]`.
    pub(super) fn handle_plugins_command(&mut self, content: &str, cx: &mut Context<Self>) -> bool {
        let mut words = content.split_whitespace();
        if !matches!(words.next(), Some("/plugins" | "/plugin")) {
            return false;
        }
        let action = words.next().unwrap_or("list");
        let name = words.next();
        let code = words.next().is_some_and(|flag| matches!(flag, "code" | "--code"));
        let item = match (action, name) {
            ("list" | "status", _) => Item::Assistant(crate::plugins::summary_markdown()),
            ("enable" | "disable", Some(name)) => {
                match crate::plugins::set_enabled(name, action == "enable") {
                    Ok(report) => Item::Assistant(format!(
                        "Plugin `{name}` {}d.\n\n{}",
                        action,
                        crate::plugins::report_markdown(&report)
                    )),
                    Err(error) => Item::Error(error),
                }
            }
            ("new", Some(name)) => match crate::plugins::create(name, code) {
                Ok(root) => Item::Assistant(if code {
                    format!(
                        "Created a code plugin at `{}`. Edit `index.mjs`, then run `/plugins reload`. Try `/standup`, and open its panel from the sidebar after restarting Desktop.",
                        root.display()
                    )
                } else {
                    format!(
                        "Created a starter plugin at `{}`. Try `/hello`, then edit its files and run `/plugins reload`.",
                        root.display()
                    )
                }),
                Err(error) => Item::Error(error),
            },
            ("reload" | "sync", _) => {
                Item::Assistant(crate::plugins::report_markdown(&crate::plugins::sync_now()))
            }
            _ => Item::Error(
                "Usage: `/plugins`, `/plugins enable <name>`, `/plugins disable <name>`, `/plugins new <name>`, `/plugins reload`.".into(),
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
