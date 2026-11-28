//! Export a conversation transcript to clean Markdown (`/export` or `/export clip`).

use super::*;
use crate::locale::{is_zh, tr};
use std::path::PathBuf;

pub(super) fn render_transcript_markdown(panel: &Panel) -> String {
    let mut out = String::new();
    let title = panel.title.trim();
    let heading = if title.is_empty() {
        tr("Conversation Export", "会话导出记录")
    } else {
        title
    };
    out.push_str(&format!("# {heading}\n\n"));
    if let Some(dir) = panel.working_dir.as_deref().filter(|d| !d.is_empty()) {
        out.push_str(&format!(
            "- **{}**: `{dir}`\n",
            tr("Working directory", "工作目录")
        ));
    }
    if let Some(model) = panel.model.as_deref().filter(|m| !m.is_empty()) {
        out.push_str(&format!("- **{}**: `{model}`\n", tr("Model", "模型")));
    }
    out.push_str(&format!(
        "- **{}**: `{}`\n\n---\n\n",
        tr("Session ID", "会话 ID"),
        panel.session_id
    ));

    let mut turn = 0usize;
    for item in panel.items.iter() {
        match item {
            Item::User(text) if !jcode_sdk::todo::is_auto_poke_message(text) => {
                turn += 1;
                let clean = super::voice::tag::strip(text).0;
                out.push_str(&format!(
                    "## {} #{turn}\n\n{}\n\n",
                    tr("User", "用户"),
                    clean.trim()
                ));
            }
            Item::Assistant(text) if !text.trim().is_empty() => {
                out.push_str(&format!(
                    "### {}\n\n{}\n\n",
                    tr("Assistant", "助手"),
                    text.trim()
                ));
            }
            Item::Tool {
                name,
                input,
                output,
                error,
                ..
            } => {
                let status = if error.is_some() { "error" } else { "ok" };
                out.push_str(&format!(
                    "<details>\n<summary>{}: <code>{name}</code> ({status})</summary>\n\n",
                    tr("Tool", "工具调用")
                ));
                if !input.trim().is_empty() {
                    out.push_str(&format!("```json\n{}\n```\n\n", input.trim()));
                }
                if let Some(err) = error {
                    out.push_str(&format!("**Error**: `{err}`\n\n"));
                } else if !output.trim().is_empty() {
                    let preview: String = output.chars().take(1200).collect();
                    out.push_str(&format!("```text\n{}\n```\n\n", preview.trim()));
                }
                out.push_str("</details>\n\n");
            }
            Item::Error(err) => {
                out.push_str(&format!("> **{}**: {err}\n\n", tr("Error", "错误")));
            }
            _ => {}
        }
    }
    out
}

impl Panel {
    pub(super) fn export_conversation(&mut self, args: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            self.items.push(Item::Error(
                tr(
                    "Nothing to export yet — start a conversation first.",
                    "当前会话暂无可导出的对话内容。",
                )
                .into(),
            ));
            cx.notify();
            return;
        }
        let markdown = render_transcript_markdown(self);
        if matches!(args.trim(), "clip" | "clipboard" | "copy") {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(markdown));
            self.items.push(Item::Assistant(
                tr(
                    "Copied the full conversation Markdown to clipboard.",
                    "已将完整对话记录以 Markdown 格式复制到剪贴板。",
                )
                .into(),
            ));
            cx.notify();
            return;
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let slug: String = self
            .title
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .chars()
            .take(32)
            .collect();
        let slug = if slug.is_empty() { "session" } else { slug.as_str() };
        let path: PathBuf = if !args.trim().is_empty() {
            let req = PathBuf::from(args.trim().trim_matches('"'));
            if req.is_absolute() {
                req
            } else if let Some(wd) = self.working_dir.as_deref().filter(|d| !d.is_empty()) {
                PathBuf::from(wd).join(req)
            } else {
                req
            }
        } else {
            crate::harness::jcode_home()
                .unwrap_or_else(std::env::temp_dir)
                .join("exports")
                .join(format!("{slug}-{stamp}.md"))
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&path, &markdown) {
            Ok(()) => {
                self.items.push(Item::Assistant(if is_zh() {
                    format!(
                        "已导出会话 Markdown 至 `{}`（也可使用 `/export clip` 直接复制到剪贴板）。",
                        path.display()
                    )
                } else {
                    format!(
                        "Exported conversation Markdown to `{}` (use `/export clip` to copy to clipboard).",
                        path.display()
                    )
                }));
                if let Some(window) = window {
                    window.dispatch_action(
                        Box::new(crate::ui_actions::OpenCodeFilePanel { path }),
                        cx,
                    );
                }
            }
            Err(error) => {
                self.items.push(Item::Error(if is_zh() {
                    format!("导出会话失败：{error}")
                } else {
                    format!("Could not export conversation: {error}")
                }));
            }
        }
        self.transcript_measurements.dirty = true;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn markdown_export_formats_turns_and_tools(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        let md = panel.update(vcx, |panel, _| {
            panel.title = "Bug Hunt".into();
            *panel.items = vec![
                Item::User("Check main.rs".into()),
                Item::Assistant("Found the issue.".into()),
            ];
            render_transcript_markdown(panel)
        });
        assert!(md.contains("# Bug Hunt"));
        assert!(md.contains("Check main.rs"));
        assert!(md.contains("Found the issue."));
    }
}
