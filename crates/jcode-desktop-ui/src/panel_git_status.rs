//! Live Git working-tree summary badge, graphical diff launcher, and `AGENTS.md`
//! project rules indicator for the session composer.

use super::*;
use crate::diff::{FileDiff, tool_diffs};
use crate::locale::{is_zh, tr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const REFRESH_THROTTLE: Duration = Duration::from_secs(6);
const MAX_UNTRACKED_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct WorkspaceGitSummary {
    pub is_git_repo: bool,
    pub changed_files: usize,
    pub added: usize,
    pub removed: usize,
    pub rules_file: Option<(String, PathBuf)>,
}

#[derive(Default)]
pub(super) struct GitStatusState {
    pub dir: Option<String>,
    pub summary: WorkspaceGitSummary,
    pub last_refresh: Option<Instant>,
    pub refreshing: bool,
    pub opening_diff: bool,
}

fn run_git(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    crate::platform::no_console_window(&mut cmd);
    cmd.output().ok()
}

pub(super) fn detect_rules_file(dir: &Path) -> Option<(String, PathBuf)> {
    for rel in ["AGENTS.md", "CLAUDE.md", ".jcode/AGENTS.md", ".devin/AGENTS.md"] {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            return Some((rel.to_owned(), candidate));
        }
    }
    None
}

pub(super) fn inspect_workspace_sync(dir: &Path) -> WorkspaceGitSummary {
    if !dir.is_dir() {
        return WorkspaceGitSummary::default();
    }
    let rules_file = detect_rules_file(dir);
    let Some(status_out) = run_git(dir, &["status", "--porcelain", "-uall"]) else {
        return WorkspaceGitSummary {
            rules_file,
            ..Default::default()
        };
    };
    if !status_out.status.success() {
        return WorkspaceGitSummary {
            rules_file,
            ..Default::default()
        };
    }
    let status_text = String::from_utf8_lossy(&status_out.stdout);
    let mut changed_files = 0usize;
    let mut untracked_added = 0usize;
    for line in status_text.lines().filter(|l| l.len() >= 3) {
        changed_files += 1;
        if let Some(rel) = line.strip_prefix("?? ") {
            let clean = rel.trim().trim_matches('"');
            let path = dir.join(clean);
            if let Ok(meta) = std::fs::metadata(&path)
                && meta.is_file()
                && meta.len() <= MAX_UNTRACKED_BYTES
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                untracked_added += text.lines().count();
            }
        }
    }
    let mut added = untracked_added;
    let mut removed = 0usize;
    if changed_files > 0 {
        let numstat = run_git(dir, &["diff", "--numstat", "HEAD"])
            .filter(|o| o.status.success())
            .or_else(|| run_git(dir, &["diff", "--numstat"]));
        if let Some(out) = numstat {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let mut parts = line.split('\t');
                if let (Some(a), Some(r)) = (parts.next(), parts.next()) {
                    added += a.parse::<usize>().unwrap_or(0);
                    removed += r.parse::<usize>().unwrap_or(0);
                }
            }
        }
    }
    WorkspaceGitSummary {
        is_git_repo: true,
        changed_files,
        added,
        removed,
        rules_file,
    }
}

pub(super) fn collect_working_tree_diffs(dir: &Path) -> Result<Vec<FileDiff>, String> {
    let diff_out = run_git(dir, &["diff", "--no-ext-diff", "HEAD"])
        .filter(|o| o.status.success())
        .or_else(|| run_git(dir, &["diff", "--no-ext-diff"]))
        .ok_or_else(|| tr("Could not run git diff.", "无法执行 git diff。").to_owned())?;
    let patch_text = String::from_utf8_lossy(&diff_out.stdout);
    let mut files = if patch_text.trim().is_empty() {
        Vec::new()
    } else {
        tool_diffs("patch", &patch_text)
    };
    if let Some(status_out) = run_git(dir, &["status", "--porcelain", "-uall"]).filter(|o| o.status.success()) {
        for line in String::from_utf8_lossy(&status_out.stdout).lines() {
            let Some(rel) = line.strip_prefix("?? ") else {
                continue;
            };
            let clean = rel.trim().trim_matches('"').replace('\\', "/");
            if files.iter().any(|f| f.path == clean) {
                continue;
            }
            let full = dir.join(&clean);
            let mut lines = Vec::new();
            if let Ok(meta) = std::fs::metadata(&full)
                && meta.is_file()
                && meta.len() <= MAX_UNTRACKED_BYTES
                && let Ok(text) = std::fs::read_to_string(&full)
            {
                let count = text.lines().count();
                if count > 0 {
                    lines.push(format!("@@ -0,0 +1,{count} @@"));
                    lines.extend(text.lines().map(|l| format!("+{l}")));
                }
            }
            files.push(FileDiff {
                path: clean,
                previous_path: None,
                kind: "Added".into(),
                lines,
            });
        }
    }
    Ok(files)
}

impl Panel {
    pub(super) fn maybe_refresh_git_status(&mut self, force: bool, cx: &mut Context<Self>) {
        if cfg!(test)
            || crate::harness::screenshot_mode()
            || self.is_side_document()
            || self.is_accounts_panel()
            || self.preview_state.is_some()
            || crate::harness::remote_host(&self.session_id).is_some()
        {
            return;
        }
        let Some(dir) = self.working_dir.clone().filter(|d| !d.trim().is_empty()) else {
            return;
        };
        let dir_changed = self.git_status.dir.as_deref() != Some(dir.as_str());
        if !force
            && !dir_changed
            && (self.git_status.refreshing
                || self
                    .git_status
                    .last_refresh
                    .is_some_and(|t| t.elapsed() < REFRESH_THROTTLE))
        {
            return;
        }
        self.git_status.dir = Some(dir.clone());
        self.git_status.refreshing = true;
        self.git_status.last_refresh = Some(Instant::now());
        let target_dir = dir.clone();
        let work = cx
            .background_executor()
            .spawn(async move { inspect_workspace_sync(Path::new(&target_dir)) });
        cx.spawn(async move |panel, cx| {
            let summary = work.await;
            let _ = panel.update(cx, |this, cx| {
                this.git_status.refreshing = false;
                if this.working_dir.as_deref() == Some(dir.as_str())
                    && this.git_status.summary != summary
                {
                    this.git_status.summary = summary;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn open_working_tree_diff(&mut self, cx: &mut Context<Self>) {
        if self.git_status.opening_diff {
            return;
        }
        let Some(dir) = self.working_dir.clone().filter(|d| !d.trim().is_empty()) else {
            self.items.push(Item::Error(
                tr(
                    "This session has no local working directory.",
                    "当前会话没有可审阅的本地工作目录。",
                )
                .into(),
            ));
            cx.notify();
            return;
        };
        self.git_status.opening_diff = true;
        let path = PathBuf::from(&dir);
        let work = cx
            .background_executor()
            .spawn(async move { collect_working_tree_diffs(&path) });
        cx.spawn(async move |panel, cx| {
            let result = work.await;
            let _ = panel.update(cx, |this, cx| {
                this.git_status.opening_diff = false;
                match result {
                    Ok(files) if files.is_empty() => {
                        this.git_status.summary.changed_files = 0;
                        this.git_status.summary.added = 0;
                        this.git_status.summary.removed = 0;
                        this.items.push(Item::Assistant(
                            tr("Working tree is clean.", "当前工作区干净，暂无未提交变更。").into(),
                        ));
                    }
                    Ok(files) => {
                        let source = if is_zh() {
                            format!("Git 工作区实时未提交改动 · {}", crate::platform_dirs::tilde(&dir))
                        } else {
                            format!("Live Git working tree · {}", crate::platform_dirs::tilde(&dir))
                        };
                        this.open_working_tree_diff_files(Arc::new(files), source, cx);
                    }
                    Err(err) => {
                        this.items.push(Item::Error(err));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_git_and_rules_pills(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        if crate::config::display().screenshot_mode
            || self.is_side_document()
            || self.is_accounts_panel()
            || crate::harness::remote_host(&self.session_id).is_some()
        {
            return Vec::new();
        }
        let Some(working_dir) = self.working_dir.as_deref().filter(|d| !d.is_empty()) else {
            return Vec::new();
        };
        if self.git_status.dir.as_deref() != Some(working_dir) {
            return Vec::new();
        }
        let theme = Theme::global();
        let summary = &self.git_status.summary;
        let mut pills = Vec::new();

        if let Some((label, rules_path)) = summary.rules_file.clone() {
            let tip_label = label.clone();
            pills.push(
                div()
                    .id("composer-rules-pill")
                    .debug_selector(|| "composer-rules-pill".into())
                    .flex_none()
                    .h(px(super::composer::TAB_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(theme.USER_BG)
                    .text_size(px(10.5))
                    .text_color(theme.TEXT_DIM)
                    .cursor_pointer()
                    .hover(|el| el.bg(theme.ACCENT_DIM).text_color(theme.TEXT))
                    .tooltip(move |_, cx| {
                        cx.new(|_| {
                            super::usage::MeterTooltip(
                                if is_zh() {
                                    format!("已加载项目规则 `{tip_label}`（点击打开查看）")
                                } else {
                                    format!("Project rules `{tip_label}` active (click to view)")
                                },
                            )
                        })
                        .into()
                    })
                    .on_click(cx.listener(move |_, _, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(
                            Box::new(crate::ui_actions::OpenCodeFilePanel {
                                path: rules_path.clone(),
                            }),
                            cx,
                        );
                    }))
                    .child(label)
                    .into_any_element(),
            );
        } else if summary.is_git_repo {
            let dir_owned = PathBuf::from(working_dir);
            pills.push(
                div()
                    .id("composer-rules-init")
                    .debug_selector(|| "composer-rules-init".into())
                    .flex_none()
                    .h(px(super::composer::TAB_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(theme.USER_BG)
                    .text_size(px(10.5))
                    .text_color(theme.TEXT_FAINT)
                    .cursor_pointer()
                    .hover(|el| el.bg(theme.ACCENT_DIM).text_color(theme.TEXT))
                    .tooltip(|_, cx| {
                        cx.new(|_| {
                            super::usage::MeterTooltip(
                                tr(
                                    "Create a starter AGENTS.md project rules file",
                                    "为此仓库初始化 AGENTS.md 项目规则文件并打开",
                                )
                                .into(),
                            )
                        })
                        .into()
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        let target = dir_owned.join("AGENTS.md");
                        if !target.exists() {
                            let starter = if is_zh() {
                                "# 项目协作与代码规范 (AGENTS.md)\n\n## 构建与测试命令\n- 在此补充本项目的常用构建、检查与测试命令\n\n## 代码与架构约定\n- 遵循仓库现有命名、目录分层与错误处理风格\n"
                            } else {
                                "# Project Instructions (AGENTS.md)\n\n## Build & Test\n- Document build, lint, and test commands here\n\n## Code Conventions\n- Follow existing naming, module structure, and error handling patterns\n"
                            };
                            let _ = std::fs::write(&target, starter);
                        }
                        this.maybe_refresh_git_status(true, cx);
                        window.dispatch_action(
                            Box::new(crate::ui_actions::OpenCodeFilePanel { path: target }),
                            cx,
                        );
                    }))
                    .child(tr("+ Rules", "+ 规则"))
                    .into_any_element(),
            );
        }

        if summary.changed_files > 0 {
            let text = if is_zh() {
                format!(
                    "{} 变更 +{} −{}",
                    summary.changed_files, summary.added, summary.removed
                )
            } else {
                format!(
                    "{} changed +{} −{}",
                    summary.changed_files, summary.added, summary.removed
                )
            };
            pills.push(
                div()
                    .id("composer-git-changes")
                    .debug_selector(|| "composer-git-changes".into())
                    .flex_none()
                    .h(px(super::composer::TAB_HEIGHT))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(theme.ACCENT_DIM)
                    .text_size(px(10.5))
                    .font_family(theme.FONT_MONO)
                    .text_color(theme.ACCENT)
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover(|el| el.bg(theme.TOOL_BG))
                    .tooltip(|_, cx| {
                        cx.new(|_| {
                            super::usage::MeterTooltip(
                                tr(
                                    "Click to review all uncommitted Git changes in a visual diff panel",
                                    "点击打开图形化 Diff 面板审阅当前工作区全部未提交改动",
                                )
                                .into(),
                            )
                        })
                        .into()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_working_tree_diff(cx);
                    }))
                    .child(text)
                    .into_any_element(),
            );
        }
        pills
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_agents_md_in_workspace() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_rules_file(dir.path()).is_none());
        std::fs::write(dir.path().join("AGENTS.md"), "# Rules\n").unwrap();
        let found = detect_rules_file(dir.path()).unwrap();
        assert_eq!(found.0, "AGENTS.md");
    }
}
