//! Slash-command catalog ported from the Jcode TUI. Keep command names and help in sync.

#[derive(Clone, Copy)]
pub(crate) struct RegisteredCommand {
    pub(crate) name: &'static str,
    pub(crate) help: &'static str,
    pub(crate) hidden: bool,
}

impl RegisteredCommand {
    const fn public(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            hidden: false,
        }
    }

    const fn remote(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            hidden: false,
        }
    }

    const fn hidden(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            hidden: true,
        }
    }
}

pub(crate) const REGISTERED_COMMANDS: &[RegisteredCommand] = &[
    RegisteredCommand::public("/help", "Show help and keyboard shortcuts"),
    RegisteredCommand::public("/?", "Show help and keyboard shortcuts"),
    RegisteredCommand::public("/commands", "Alias for /help"),
    RegisteredCommand::public("/model", "List or switch models"),
    RegisteredCommand::public("/models", "Alias for /model"),
    RegisteredCommand::public(
        "/provider-test-coverage",
        "Show live-test evidence for the current provider/model",
    ),
    RegisteredCommand::hidden("/model-status", "Alias for /provider-test-coverage"),
    RegisteredCommand::public("/refresh-model-list", "Refresh provider model catalogs"),
    RegisteredCommand::public("/agents", "Configure models for agent roles"),
    RegisteredCommand::public(
        "/swarm-prompt",
        "Open the active swarm routing prompt in your editor",
    ),
    RegisteredCommand::public("/subagent", "Launch a subagent manually"),
    RegisteredCommand::public("/observe", "Show the latest tool context in the side panel"),
    RegisteredCommand::public("/todos", "Show the session todo list as a card in the chat"),
    RegisteredCommand::hidden("/todo", "Alias for /todos"),
    RegisteredCommand::public("/splitview", "Mirror the current chat in the side panel"),
    RegisteredCommand::public("/split-view", "Alias for /splitview"),
    RegisteredCommand::public("/btw", "Ask a side question in the side panel"),
    RegisteredCommand::public("/ssh", "Connect to a remote machine using system SSH"),
    RegisteredCommand::public("/git", "Show git status for the session working directory"),
    RegisteredCommand::public(
        "/plugins",
        "List, enable, disable or create plugins (commands, skills, hooks, MCP, applets)",
    ),
    RegisteredCommand::public("/extensions", "Manage MCP servers, skills and plugins"),
    RegisteredCommand::public("/mcp", "Manage MCP servers (add, edit, enable, test)"),
    RegisteredCommand::public("/colors", "List or change Desktop color themes"),
    RegisteredCommand::hidden("/color", "Alias for /colors"),
    RegisteredCommand::public("/hotkeys", "List registered Desktop hotkeys and contexts"),
    RegisteredCommand::public("/terminal-setup", "Explain Desktop's built-in terminal setup"),
    RegisteredCommand::public("/commit", "Make logical commits from current changes"),
    RegisteredCommand::public(
        "/commit-push",
        "Make logical commits from current changes, then push",
    ),
    RegisteredCommand::hidden("/commit-and-push", "Alias for /commit-push"),
    RegisteredCommand::public(
        "/publish",
        "Desktop selfdev: commit, push, release, and publish, tracked in a new panel",
    ),
    RegisteredCommand::public(
        "/fast-release",
        "Open Desktop's standard validated publish tracker",
    ),
    RegisteredCommand::public(
        "/fast-macos-release",
        "Open Desktop's cross-platform publish tracker",
    ),
    RegisteredCommand::public("/remote", "Reach this session from another machine"),
    RegisteredCommand::public(
        "/remote-release",
        "Open Desktop's publish tracker with CI-managed platform builds",
    ),
    RegisteredCommand::hidden("/cut-release", "Alias for /fast-release"),
    RegisteredCommand::hidden("/commit-push-release", "Alias for /cut-release"),
    RegisteredCommand::public(
        "/triage",
        "Triage new GitHub issues and autonomously fix the safe ones",
    ),
    RegisteredCommand::public("/transcript", "Open the current session transcript file"),
    RegisteredCommand::public("/subagent-model", "Show/change subagent model policy"),
    RegisteredCommand::public("/autoreview", "Show/toggle automatic end-of-turn review"),
    RegisteredCommand::public("/autojudge", "Show/toggle automatic end-of-turn judging"),
    RegisteredCommand::public("/review", "Launch a one-shot headed review session"),
    RegisteredCommand::public("/judge", "Launch a one-shot headed judge session"),
    RegisteredCommand::public(
        "/effort",
        "Set reasoning effort (none/minimal/low/medium/high/xhigh/max)",
    ),
    RegisteredCommand::public("/fast", "Toggle fast mode"),
    RegisteredCommand::public("/transport", "Show/change connection transport"),
    RegisteredCommand::public("/alignment", "Show/change default text alignment"),
    RegisteredCommand::public(
        "/compact-notifications",
        "Show/toggle single-line swarm/file-activity notifications",
    ),
    RegisteredCommand::public(
        "/show-agentgrep-output",
        "Show/toggle full agentgrep search output inline in chat",
    ),
    RegisteredCommand::public(
        "/tool-call-details",
        "Show/toggle dimmed technical details on tool rows with an intent",
    ),
    RegisteredCommand::public(
        "/thinking-display",
        "Show/fold the model's thinking text (off/full/current/collapsed/toggle)",
    ),
    RegisteredCommand::public(
        "/steps",
        "Fold each turn's tool calls, thoughts, and notices into one pill (collapsed/expanded/toggle)",
    ),
    RegisteredCommand::hidden("/thinking", "Alias for /thinking-display"),
    RegisteredCommand::hidden("/reasoning", "Alias for /thinking-display"),
    RegisteredCommand::public("/cancel", "Cancel the current prompt or operation"),
    RegisteredCommand::public("/clear", "Clear conversation history"),
    RegisteredCommand::public("/cls", "Clear the view only, keeping context"),
    RegisteredCommand::hidden("/clear-view", "Alias for /cls"),
    RegisteredCommand::public("/rewind", "Rewind conversation to previous message"),
    RegisteredCommand::public("/poke", "Poke model to resume with incomplete todos"),
    RegisteredCommand::public("/plan", "Create a plan-only response as a plan card"),
    RegisteredCommand::public("/improve", "Autonomously improve the repository"),
    RegisteredCommand::public("/refactor", "Run a safe refactor loop"),
    RegisteredCommand::public("/compact", "Compact context"),
    RegisteredCommand::public("/fix", "Recover when the model cannot continue"),
    RegisteredCommand::public("/dictate", "Run configured external dictation command"),
    RegisteredCommand::public("/dictation", "Alias for /dictate"),
    RegisteredCommand::public("/memory", "Toggle memory feature"),
    RegisteredCommand::public("/test", "Verify a claim/current changes with layered tests"),
    RegisteredCommand::public(
        "/initiatives",
        "Show current session goals and explain retired legacy initiatives",
    ),
    RegisteredCommand::public("/goals", "Legacy alias for /initiatives"),
    RegisteredCommand::public("/swarm", "Toggle swarm feature"),
    RegisteredCommand::public("/overnight", "Run a supervised overnight coordinator"),
    RegisteredCommand::public("/context", "Show the full session context snapshot"),
    RegisteredCommand::public(
        "/skills",
        "Manage skills (bare) or show loaded skills (`/skills <args>`)",
    ),
    RegisteredCommand::public("/version", "Show current version"),
    RegisteredCommand::public("/changelog", "Show recent changes in this build"),
    RegisteredCommand::public("/applets", "Open the applet component showcase"),
    RegisteredCommand::public("/info", "Show session info and tokens"),
    RegisteredCommand::public(
        "/reset",
        "Review and confirm an OpenAI or Claude usage limit reset",
    ),
    RegisteredCommand::public("/usage", "Show connected provider usage limits"),
    RegisteredCommand::public(
        "/productivity",
        "Generate a shareable usage report + dashboard image",
    ),
    RegisteredCommand::public("/wrapped", "Alias for /productivity"),
    RegisteredCommand::public("/feedback", "Send feedback about jcode"),
    RegisteredCommand::public("/telemetry", "Show or change what jcode sends"),
    RegisteredCommand::public("/support", "Email support with diagnostics prefilled"),
    RegisteredCommand::public("/subscription", "Show jcode subscription status"),
    RegisteredCommand::public("/subscribe", "Why and how to subscribe to jcode"),
    RegisteredCommand::public("/config", "Show or edit configuration"),
    RegisteredCommand::public("/log", "Mark the current location in the jcode logs"),
    RegisteredCommand::public(
        "/keys",
        "List Desktop keybindings and contexts (/keys refresh to refresh the list)",
    ),
    RegisteredCommand::hidden("/keybindings", "Alias for /keys"),
    RegisteredCommand::public(
        "/diff",
        "Cycle or set diff display mode (off/inline/full/pinned/file)",
    ),
    RegisteredCommand::public(
        "/onboarding-preview",
        "Open real first-run Desktop in a separate fresh-profile window (Alt+9)",
    ),
    RegisteredCommand::public(
        "/onboarding-sim",
        "Open real first-run Desktop with an isolated fresh profile (Alt+9)",
    ),
    RegisteredCommand::public("/reload", "Reload into newest available binary"),
    RegisteredCommand::public("/restart", "Restart with current binary"),
    RegisteredCommand::public(
        "/restart-all",
        "Restart the server and every Desktop window, restoring all windows and sessions",
    ),
    RegisteredCommand::public("/rebuild", "Background rebuild and auto reload"),
    RegisteredCommand::public("/selfdev", "Open a new self-dev jcode session"),
    RegisteredCommand::public("/update", "Background update and auto reload"),
    RegisteredCommand::public("/resume", "Open session picker"),
    RegisteredCommand::public("/sessions", "Alias for /resume"),
    RegisteredCommand::public("/session", "Alias for /resume"),
    RegisteredCommand::public("/active", "Manage live sessions (working vs ready)"),
    RegisteredCommand::public("/catchup", "Open Catch Up picker"),
    RegisteredCommand::public("/back", "Return to the previous Catch Up session"),
    RegisteredCommand::public("/save", "Bookmark session for easy access"),
    RegisteredCommand::public("/unsave", "Remove bookmark from session"),
    RegisteredCommand::public("/rename", "Rename current session"),
    RegisteredCommand::public("/fork", "Fork session into a new window (optional prompt)"),
    RegisteredCommand::hidden("/split", "Alias for /fork"),
    RegisteredCommand::public("/transfer", "Compact context into a fresh handoff session"),
    RegisteredCommand::public("/workspace", "Niri-style session workspace"),
    RegisteredCommand::public("/quit", "Exit jcode"),
    RegisteredCommand::public("/auth", "Show authentication status"),
    RegisteredCommand::public("/login", "Login to a provider"),
    RegisteredCommand::public("/logout", "Log out of a provider"),
    RegisteredCommand::public("/account", "Open the combined account picker"),
    RegisteredCommand::public("/accounts", "Alias for /account"),
    RegisteredCommand::public(
        "/api",
        "Add or manage custom OpenAI-compatible third-party API endpoints",
    ),
    RegisteredCommand::public("/custom-api", "Alias for /api"),
    RegisteredCommand::public("/cache", "Show cache stats or set cache TTL"),
    RegisteredCommand::public("/debug-visual", "Toggle visual debug overlay"),
    RegisteredCommand::public("/screenshot-mode", "Hide identifying interface metadata"),
    RegisteredCommand::public("/screenshot", "Export a private JSON screenshot debug snapshot"),
    RegisteredCommand::public("/record", "Record a bounded debug snapshot timeline (JSONL)"),
    RegisteredCommand::public("/export", "Export conversation transcript to Markdown (or `/export clip`)"),
    RegisteredCommand::public("/git-review", "Open visual diff review for uncommitted Git changes"),
    RegisteredCommand::remote("/client-reload", "Rebuild and hot-reload the Desktop UI"),
    RegisteredCommand::remote("/server-reload", "Request a graceful server binary reload"),
    RegisteredCommand::remote(
        "/continue",
        "Continue every interrupted live session that would auto-resume",
    ),
    RegisteredCommand::remote("/resumeall", "Alias for /continue"),
    RegisteredCommand::hidden("/resume-all", "Alias for /continue"),
    RegisteredCommand::hidden("/z", "Secret premium-mode command"),
    RegisteredCommand::hidden("/zz", "Secret premium-mode command"),
    RegisteredCommand::hidden("/zzz", "Secret premium-mode command"),
    RegisteredCommand::hidden("/zstatus", "Secret premium-mode status command"),
];

/// Every non-hidden slash command with its one-line description, in
/// registration order. The `/help` overlay uses this to list commands its
/// hand-written sections have not covered, so a newly registered command can
/// never be invisible to users.
pub(crate) fn registered_command_entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    REGISTERED_COMMANDS
        .iter()
        .filter(|command| !command.hidden)
        .map(|command| (command.name, command.help))
}

pub(crate) fn registered_command(name: &str) -> Option<RegisteredCommand> {
    REGISTERED_COMMANDS
        .iter()
        .copied()
        .find(|command| command.name == name)
}

pub(crate) fn localized_command_help(name: &str, fallback: &'static str) -> &'static str {
    localized_command_help_for_locale(name, fallback, crate::locale::is_zh())
}

pub(crate) fn localized_command_help_for_locale(
    name: &str,
    fallback: &'static str,
    zh: bool,
) -> &'static str {
    if !zh {
        return fallback;
    }
    match name {
        "/help" | "/?" => "显示帮助与快捷键",
        "/commands" => "/help 的别名",
        "/model" => "查看或切换模型",
        "/models" => "/model 的别名",
        "/provider-test-coverage" => "查看当前线路与模型的实测验证覆盖情况",
        "/refresh-model-list" => "刷新各线路的模型列表",
        "/agents" => "为各代理角色配置模型",
        "/swarm-prompt" => "在编辑器中打开当前多代理协作路由提示词",
        "/subagent" => "手动启动一个子代理",
        "/observe" => "在侧栏显示最新工具调用上下文",
        "/todos" => "在聊天中以卡片形式显示当前会话待办列表",
        "/splitview" => "在侧栏镜像显示当前对话",
        "/split-view" => "/splitview 的别名",
        "/btw" => "在侧栏中发起旁路提问",
        "/ssh" => "使用系统 SSH 连接到远程主机",
        "/git" => "显示当前会话工作目录的 Git 状态",
        "/plugins" => "列出、启用、禁用或创建插件（命令、技能、钩子、MCP、小程序）",
        "/extensions" => "管理 MCP 服务、技能与插件",
        "/mcp" => "管理 MCP 服务（添加、编辑、启用、测试）",
        "/colors" => "查看或切换桌面端配色主题",
        "/hotkeys" => "列出已注册的桌面端快捷键及作用域",
        "/terminal-setup" => "说明桌面端内置终端的配置方式",
        "/commit" => "根据当前改动生成逻辑清晰的提交",
        "/commit-push" => "根据当前改动生成逻辑清晰的提交并推送",
        "/publish" => "桌面端自研发布：提交、推送、发布并在新面板中跟踪",
        "/fast-release" => "打开桌面端标准验证发布跟踪器",
        "/fast-macos-release" => "打开桌面端跨平台发布跟踪器",
        "/remote" => "从其他设备远程访问当前会话",
        "/remote-release" => "打开基于 CI 构建的桌面端发布跟踪器",
        "/triage" => "分拣新的 GitHub Issue 并自动修复安全的问题",
        "/transcript" => "打开当前会话的完整记录文件",
        "/subagent-model" => "查看或更改子代理模型策略",
        "/autoreview" => "查看或开关回合结束自动审查",
        "/autojudge" => "查看或开关回合结束自动评审",
        "/review" => "启动一次带界面的单次审查会话",
        "/judge" => "启动一次带界面的单次评审会话",
        "/effort" => "设置推理强度（none/minimal/low/medium/high/xhigh/max）",
        "/fast" => "切换快速模式",
        "/transport" => "查看或切换连接传输方式",
        "/alignment" => "查看或更改默认文本对齐方式",
        "/compact-notifications" => "查看或开关单行多代理/文件活动通知",
        "/show-agentgrep-output" => "查看或开关在聊天中内联展开 agentgrep 搜索输出",
        "/tool-call-details" => "查看或开关工具行上的淡色技术细节",
        "/thinking-display" => "展开或折叠模型思考过程（off/full/current/collapsed/toggle）",
        "/steps" => "将每轮工具调用、思考与提示折叠为单个胶囊（collapsed/expanded/toggle）",
        "/cancel" => "取消当前提示词或正在进行的操作",
        "/clear" => "清空对话历史",
        "/cls" => "仅清空视图显示，保留上下文",
        "/rewind" => "将对话回退到上一条消息",
        "/poke" => "提醒模型继续处理未完成的待办事项",
        "/plan" => "仅生成方案并以计划卡片形式展示",
        "/improve" => "自主改进当前仓库",
        "/refactor" => "运行安全重构循环",
        "/compact" => "压缩会话上下文",
        "/fix" => "在模型无法继续时尝试恢复",
        "/dictate" => "运行已配置的外部语音听写命令",
        "/dictation" => "/dictate 的别名",
        "/memory" => "切换记忆功能",
        "/test" => "使用分层测试验证结论或当前改动",
        "/initiatives" => "显示当前会话目标并说明已退役的旧版倡议",
        "/goals" => "/initiatives 的旧版别名",
        "/swarm" => "切换多代理协作功能",
        "/overnight" => "运行受监督的通宵协调任务",
        "/context" => "显示完整的会话上下文快照",
        "/skills" => "管理技能（无参数）或查看已加载技能（`/skills <参数>`）",
        "/version" => "显示当前版本",
        "/changelog" => "显示当前构建的近期更新",
        "/applets" => "打开小程序组件展示面板",
        "/info" => "显示会话信息与 token 用量",
        "/reset" => "查看并确认重置 OpenAI 或 Claude 用量额度",
        "/usage" => "显示已连接线路的用量限额",
        "/productivity" => "生成可分享的用量报告与仪表盘图片",
        "/wrapped" => "/productivity 的别名",
        "/feedback" => "发送关于 jcode 的反馈",
        "/telemetry" => "查看或更改 jcode 发送的遥测数据",
        "/support" => "发送预填诊断信息的支持邮件",
        "/subscription" => "查看 jcode 订阅状态",
        "/subscribe" => "了解订阅 jcode 的原因与方式",
        "/config" => "查看或编辑配置",
        "/log" => "在 jcode 日志中标记当前位置",
        "/keys" => "列出桌面端快捷键与作用域（`/keys refresh` 可刷新）",
        "/diff" => "切换或设置差异展示模式（off/inline/full/pinned/file）",
        "/export" => "将当前对话导出为 Markdown 文件（`/export clip` 复制到剪贴板）",
        "/git-review" => "打开图形化面板审阅当前工作区全部未提交 Git 改动",
        "/onboarding-preview" => "在独立新配置窗口中打开首次运行引导预览（Alt+9）",
        "/onboarding-sim" => "使用隔离的新配置打开首次运行引导桌面端（Alt+9）",
        "/reload" => "热重载到最新可用的二进制版本",
        "/restart" => "使用当前二进制重启",
        "/restart-all" => "重启服务端及所有桌面端窗口，并恢复全部窗口与会话",
        "/rebuild" => "后台重新构建并自动重载",
        "/selfdev" => "打开新的 jcode 自开发会话",
        "/update" => "后台更新并自动重载",
        "/resume" => "打开会话选择器",
        "/sessions" | "/session" => "/resume 的别名",
        "/active" => "管理活跃会话（运行中与待命）",
        "/catchup" => "打开进度回顾选择器",
        "/back" => "返回上一个进度回顾会话",
        "/save" => "收藏当前会话以便快速访问",
        "/unsave" => "取消收藏当前会话",
        "/rename" => "重命名当前会话",
        "/fork" => "将当前会话分叉到新窗口（可选附带提示词）",
        "/transfer" => "将上下文压缩并交接到新会话",
        "/workspace" => "Niri 风格会话工作区",
        "/quit" => "退出 jcode",
        "/auth" => "显示认证状态",
        "/login" => "登录到指定线路",
        "/logout" => "从指定线路登出",
        "/account" => "打开综合账户选择器",
        "/accounts" => "/account 的别名",
        "/api" => "添加或管理兼容 OpenAI 的第三方自定义 API 端点",
        "/custom-api" => "/api 的别名",
        "/cache" => "查看缓存统计或设置缓存 TTL",
        "/debug-visual" => "开关可视化调试浮层",
        "/screenshot-mode" => "隐藏可识别身份的界面元数据",
        "/screenshot" => "导出脱敏的 JSON 截图调试快照",
        "/record" => "录制有界调试快照时间线（JSONL）",
        "/client-reload" => "重新构建并热重载桌面端 UI",
        "/server-reload" => "请求平滑重载服务端二进制",
        "/continue" => "继续所有可自动恢复的中断实时会话",
        "/resumeall" => "/continue 的别名",
        _ => fallback,
    }
}

fn help_markdown_with(
    zh: bool,
    resolve_help: impl Fn(&str, &'static str) -> &'static str,
) -> String {
    let mut help = String::from(if zh {
        "**命令列表**\n\n"
    } else {
        "**Commands**\n\n"
    });
    for (name, description) in registered_command_entries() {
        let description = resolve_help(name, description);
        help.push_str(&format!("- `{name}` — {description}\n"));
    }
    help
}

pub(crate) fn help_markdown() -> String {
    help_markdown_with(crate::locale::is_zh(), localized_command_help)
}

#[cfg(test)]
fn help_markdown_for_locale(zh: bool) -> String {
    help_markdown_with(zh, |name, fallback| {
        localized_command_help_for_locale(name, fallback, zh)
    })
}

/// Commands that open the session browser instead of reaching the runtime.
pub(crate) fn is_resume_command(content: &str) -> bool {
    matches!(content.trim(), "/resume" | "/sessions" | "/session")
}

/// Both account aliases open Desktop's native picker, not a runtime prompt.
pub(crate) fn is_accounts_command(content: &str) -> bool {
    matches!(content.trim(), "/account" | "/accounts")
}

/// Opens Accounts directly on the custom third-party OpenAI-compatible API form.
pub(crate) fn is_custom_api_command(content: &str) -> bool {
    matches!(content.trim(), "/api" | "/custom-api")
}

/// Target of a `/reset` command typed in the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetCommand {
    /// Bare `/reset`: the provider of the panel's current session.
    Current,
    OpenAi,
    Claude,
}

/// Parse `/reset`, `/reset <provider>` and the TUI spelling
/// `/reset usage limits <provider>`. Desktop always opens the read-only
/// review dialog, so the TUI's `confirm` and `cancel` suffixes are accepted
/// and only ever open that review. Nothing is spent without the dialog's button.
pub(crate) fn reset_command(content: &str) -> Option<ResetCommand> {
    let words: Vec<&str> = content.split_whitespace().collect();
    let rest = match words.as_slice() {
        ["/reset", "usage", "limits", rest @ ..] if !rest.is_empty() => rest,
        ["/reset", "usage", ..] => return None,
        ["/reset", rest @ ..] => rest,
        _ => return None,
    };
    let (provider, suffix) = match rest {
        [] => return Some(ResetCommand::Current),
        [provider] => (*provider, None),
        [provider, suffix] => (*provider, Some(*suffix)),
        _ => return None,
    };
    if !matches!(suffix, None | Some("confirm" | "cancel")) {
        return None;
    }
    match provider.to_ascii_lowercase().as_str() {
        "openai" | "codex" | "chatgpt" => Some(ResetCommand::OpenAi),
        "claude" | "anthropic" => Some(ResetCommand::Claude),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_command_accepts_desktop_and_tui_spellings() {
        assert_eq!(reset_command("/reset"), Some(ResetCommand::Current));
        assert_eq!(reset_command(" /reset claude "), Some(ResetCommand::Claude));
        assert_eq!(reset_command("/reset Anthropic"), Some(ResetCommand::Claude));
        assert_eq!(
            reset_command("/reset usage limits claude"),
            Some(ResetCommand::Claude)
        );
        assert_eq!(
            reset_command("/reset usage limits claude confirm"),
            Some(ResetCommand::Claude)
        );
        assert_eq!(
            reset_command("/reset usage limits openai"),
            Some(ResetCommand::OpenAi)
        );
        assert_eq!(reset_command("/reset codex"), Some(ResetCommand::OpenAi));
        for other in [
            "/resetting",
            "/reset gemini",
            "/reset claude now",
            "/reset usage limits",
            "/reset usage",
            "reset claude",
        ] {
            assert_eq!(reset_command(other), None, "{other}");
        }
        assert!(registered_command("/reset").is_some());
    }

    #[test]
    fn account_aliases_match_complete_commands_only() {
        for command in ["/account", "/accounts", "  /accounts\n", "\t/account "] {
            assert!(is_accounts_command(command));
            assert!(registered_command(command.trim()).is_some());
        }
        for command in ["/accounting", "/accounts-extra", "/account foo", "account"] {
            assert!(!is_accounts_command(command));
        }
    }

    #[test]
    fn desktop_catalog_contains_the_full_tui_command_surface() {
        assert!(REGISTERED_COMMANDS.len() >= 100);
        for command in [
            "/help", "/model", "/agents", "/commit", "/effort", "/clear", "/rewind", "/compact",
            "/test", "/resume", "/rename", "/auth", "/quit",
        ] {
            assert!(registered_command(command).is_some(), "missing {command}");
        }
        assert!(!registered_command_entries().any(|(name, _)| name == "/zzz"));
    }

    #[test]
    fn localized_command_help_covers_public_commands_in_zh_and_preserves_english_fallback() {
        for command in REGISTERED_COMMANDS {
            assert_eq!(
                localized_command_help_for_locale(command.name, command.help, false),
                command.help,
                "English fallback changed for {}",
                command.name
            );
        }
        let public_entries: Vec<_> = registered_command_entries().collect();
        let covered = public_entries
            .iter()
            .filter(|(name, fallback)| {
                localized_command_help_for_locale(name, fallback, true) != *fallback
            })
            .count();
        assert!(
            covered >= 50,
            "expected at least 50 localized commands, found {covered}"
        );
        assert_eq!(covered, public_entries.len());
        for (name, expected_zh) in [
            ("/model", "查看或切换模型"),
            ("/transport", "查看或切换连接传输方式"),
            ("/effort", "设置推理强度（none/minimal/low/medium/high/xhigh/max）"),
            ("/agents", "为各代理角色配置模型"),
            ("/subagent", "手动启动一个子代理"),
            ("/resume", "打开会话选择器"),
            ("/compact", "压缩会话上下文"),
            ("/mcp", "管理 MCP 服务（添加、编辑、启用、测试）"),
            ("/plugins", "列出、启用、禁用或创建插件（命令、技能、钩子、MCP、小程序）"),
            ("/skills", "管理技能（无参数）或查看已加载技能（`/skills <参数>`）"),
            ("/account", "打开综合账户选择器"),
            ("/api", "添加或管理兼容 OpenAI 的第三方自定义 API 端点"),
            ("/reload", "热重载到最新可用的二进制版本"),
            ("/screenshot", "导出脱敏的 JSON 截图调试快照"),
        ] {
            let fallback = registered_command(name).expect("command registered").help;
            assert_eq!(
                localized_command_help_for_locale(name, fallback, true),
                expected_zh
            );
        }
        assert_eq!(
            localized_command_help_for_locale("/unknown-cmd", "Fallback help", true),
            "Fallback help"
        );
        assert_eq!(
            localized_command_help_for_locale("/unknown-cmd", "Fallback help", false),
            "Fallback help"
        );
    }

    #[test]
    fn help_markdown_renders_both_english_and_chinese_titles_and_entries() {
        let en = help_markdown_for_locale(false);
        assert!(en.starts_with("**Commands**\n\n"));
        assert!(en.contains("- `/help` — Show help and keyboard shortcuts\n"));
        assert!(en.contains("- `/model` — List or switch models\n"));
        assert!(en.contains("- `/reload` — Reload into newest available binary\n"));

        let zh = help_markdown_for_locale(true);
        assert!(zh.starts_with("**命令列表**\n\n"));
        assert!(!zh.contains("**Commands**"));
        assert!(zh.contains("- `/help` — 显示帮助与快捷键\n"));
        assert!(zh.contains("- `/model` — 查看或切换模型\n"));
        assert!(zh.contains("- `/mcp` — 管理 MCP 服务（添加、编辑、启用、测试）\n"));
        assert!(zh.contains("- `/api` — 添加或管理兼容 OpenAI 的第三方自定义 API 端点\n"));
        assert!(zh.contains("- `/reload` — 热重载到最新可用的二进制版本\n"));
    }
}
