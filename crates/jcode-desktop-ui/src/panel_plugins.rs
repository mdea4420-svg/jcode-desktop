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
            ("new", Some(name)) => match crate::plugins::create(name) {
                Ok(root) => Item::Assistant(format!(
                    "Created a starter plugin at `{}`. Try `/hello`, then edit its files and run `/plugins reload`.",
                    root.display()
                )),
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

    /// Send a plugin command's expanded template as the prompt.
    pub(super) fn handle_plugin_slash_command(&mut self, content: &str, cx: &mut Context<Self>) -> bool {
        let Some(prompt) = crate::plugins::expand(content) else {
            return false;
        };
        self.submit_or_queue(prompt, Vec::new(), true, cx);
        true
    }
}
