# Jcode 插件（fork 版）

一个插件就是 `~/.jcode/plugins/<名字>/` 下的一个文件夹，结构和 Claude Code 插件一样，所以现有的 Claude Code 插件大多可以直接放进来用。

```text
my-plugin/
  .claude-plugin/plugin.json   名字、版本、描述（可省略，省略时用文件夹名）
  commands/*.md                斜杠命令（提示词模板）
  skills/<技能>/SKILL.md       技能
  hooks/hooks.json             事件钩子
  .mcp.json                    MCP 工具服务器
  applets/<id>/applet.json     Jcode Desktop 面板（Applet）
```

## 快速开始

在 Desktop 输入框里：

- `/plugins`：列出插件、状态、命令和技能
- `/plugins new my-tools`：生成一个可运行的示例插件（带 `/hello` 命令和一个技能）
- `/plugins enable <名字>` / `/plugins disable <名字>`
- `/plugins reload`：改了插件文件后重新同步

命令行（`jcode-plugins.exe` 和 `jcode-desktop.exe` 在同一个文件夹）：

```text
jcode-plugins list | info <名字> | enable <名字> | disable <名字>
jcode-plugins install <文件夹路径 | git 地址>
jcode-plugins uninstall <名字>
jcode-plugins new <名字>
jcode-plugins sync
jcode-plugins run "/review src"     # 打印命令展开后的提示词，方便调试
```

示例插件：`examples/plugins/safety-guard`（拦截危险命令，并提供 `/review`、`/commit`）。
安装：`jcode-plugins install <仓库>/examples/plugins/safety-guard`。

## 斜杠命令 `commands/*.md`

```markdown
---
description: 审查未提交的改动
argument-hint: [路径]
---
审查当前改动，重点看：$ARGUMENTS
```

- 文件名就是命令名：`review.md` → `/review`，同时永远可以用 `/<插件名>:review`。
- 子文件夹表示命名空间：`commands/git/commit.md` → `/git:commit`。
- 占位符：`$ARGUMENTS`（全部参数）、`$1`…`$9`（按空格拆分，支持引号）、`${CLAUDE_PLUGIN_ROOT}`（插件目录）。
- 没写占位符时，参数会附加在模板末尾。
- 和内置命令同名、或多个插件同名时，短名字不生效，只能用 `/<插件名>:<命令>`。

## 钩子 `hooks/hooks.json`

两种写法都支持，可以混用。

**Claude Code 格式**（自动翻译）：

```json
{"hooks": {"PreToolUse": [{"matcher": "Bash|Write", "hooks": [{"type": "command", "command": "bash \"${CLAUDE_PLUGIN_ROOT}/hooks/guard.sh\"", "timeout": 5}]}]}}
```

| Claude Code 事件 | 对应 jcode 事件 | 能做什么 |
|---|---|---|
| `PreToolUse` | `pre_tool` | 退出码 2，或输出 `{"decision":"block"}` / `permissionDecision: "deny"`，就会拦截这次工具调用 |
| `PostToolUse` | `post_tool_feedback` | 退出码 2 时的 stderr，或 `additionalContext`，会附加到工具结果里给模型看 |
| `UserPromptSubmit` | `turn_start` | 只能观察 |
| `Stop` | `turn_end` | 只能观察 |
| `SessionStart` / `SessionEnd` | `session_start` / `session_end` | 只能观察 |

钩子从 stdin 收到 Claude Code 格式的 JSON（`tool_name` 用 `Bash`、`Edit` 这类名字，另有 `tool_input`、`cwd` 等字段）。暂不支持的事件（`Notification`、`PreCompact`）会在 `jcode-plugins info` 里提示。

**jcode 原生格式**：键名直接用 jcode 事件名（`pre_tool`、`pre_tool_transform`、`post_tool`、`post_tool_feedback`、`turn_start`、`turn_end`、`session_start`、`session_end`），环境变量和 stdin 按 jcode 原样传入（见 jcode 的 `docs/HOOKS.md`）。只有 `pre_tool_transform` 能改写工具参数。

```json
{"hooks": {"turn_end": ["python \"${CLAUDE_PLUGIN_ROOT}/notify.py\""]}}
```

钩子命令通过 shell 执行：Windows 上优先用 Git Bash，没有就用 `cmd`；其他系统用 `sh`。也可以用环境变量 `JCODE_PLUGINS_SHELL` 指定。运行时会提供环境变量 `CLAUDE_PLUGIN_ROOT`、`JCODE_PLUGIN_ROOT` 和 `CLAUDE_PROJECT_DIR`。

钩子出错（退出码不是 0 或 2、超时、程序找不到）一律放行，不会把会话卡死。

## 技能、MCP、面板

- 技能：每个技能会被链接到 `~/.jcode/skills/plugin-<插件>-<技能>`，jcode 会照常按对话内容自动加载。
- MCP：`.mcp.json` 里的服务器会合并进 `~/.jcode/mcp.json`，名字是 `<插件>-<服务器>`。
- 面板：`applets/` 和 `~/.jcode/applets` 的格式完全一样，Desktop 启动时自动加载。

## 同步是怎么工作的

Desktop 启动时（或者运行 `jcode-plugins sync` 时）会把启用的插件写进 jcode 的配置：

- `~/.jcode/config.toml` 的 `[hooks]` 里，每个用到的事件加一条 `jcode-plugins hook <事件>`，由它分发给各个插件。这条会追加在你自己配置的钩子后面，不会覆盖。
- 只改动自己加过的条目，记录在 `~/.jcode/plugins/.state.json`。停用插件时会把它们精确地删掉，你自己的配置和插件文件都不会动。
- 新的技能、MCP 和钩子对新会话生效。

Claude Code 已安装的插件（`~/.claude/plugins`）也会被发现，但默认是关的，需要用 `/plugins enable` 打开。它们的技能 jcode 本来就会加载。
