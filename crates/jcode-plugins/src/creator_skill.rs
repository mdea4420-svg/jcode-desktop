//! Built-in `jcode-plugin-creator` skill and self-authoring prompt builder.
//!
//! Enables Jcode to author, validate, modify, and enable plugins for itself
//! inside `~/.jcode/plugins/<name>/` using either declarative files or
//! single-file Node ESM (`index.mjs`).

use std::path::{Path, PathBuf};

use crate::paths::Layout;
use crate::sync::write_atomic;

pub const SKILL_NAME: &str = "jcode-plugin-creator";

pub const SKILL_MARKDOWN: &str = r#"---
name: jcode-plugin-creator
description: 为 Jcode Desktop 创建、修改、校验并启用本地插件（Tier-0 声明式 commands/skills/hooks/mcp，或 Tier-1 Node ESM 代码插件 index.mjs + 侧栏交互面板）。当用户说「给自己加个插件 / 帮我写个插件 / 做一个斜杠命令或侧栏工具面板 / 把某套工作流做成插件 / 改一下某个插件」，或涉及 ~/.jcode/plugins 插件开发时立即调用本技能。
---

# Jcode 插件自扩展开发指南 (`jcode-plugin-creator`)

你可以直接为自己（Jcode Desktop）编写、修改并启用插件！所有本地自建插件均存放在 `~/.jcode/plugins/<kebab-name>/` 目录下，无需重新编译客户端即可热加载生效。

## 1. 先选插件层级（Tier-0 vs Tier-1）

1. **Tier-0 声明式插件**（纯 Markdown / JSON，零运行时依赖）：
   - 适合：封装固定提示词模板斜杠命令（`commands/*.md`）、专属技能（`skills/<id>/SKILL.md`）、Shell/Python 钩子（`hooks/hooks.json`）或现有 MCP 服务（`.mcp.json`）。
2. **Tier-1 ESM 代码插件**（`"main": "index.mjs"`，由本机 Node.js 驱动，功能最强大）：
   - 适合：动态斜杠命令（可执行本地 git/文件统计后直接回显聊天或生成动态提示词）、给 AI 新增可调用工具（`jcode.tool`，自动挂载为 MCP 工具）、精细化工具拦截/改写（`jcode.on`）、或**自定义图形化侧栏/独立面板（`jcode.panel`）**。

## 2. 标准目录结构

```text
~/.jcode/plugins/<plugin-name>/
  .claude-plugin/plugin.json   # 必填元数据（name, version, description, 代码插件加 "main": "index.mjs"）
  index.mjs                    # [Tier-1 可选] Node ESM 入口，export default function (jcode) { ... }
  jcode.d.ts                   # [Tier-1 推荐] TypeScript 类型提示文件
  commands/<cmd>.md            # [Tier-0 可选] 提示词斜杠命令模板（支持 $ARGUMENTS, $1..$9）
  skills/<skill-id>/SKILL.md   # [可选] 随插件分发的自动技能
  hooks/hooks.json             # [可选] 声明式事件钩子
  .mcp.json                    # [可选] 声明式 MCP 服务器（{"mcpServers": {...}}）
```

### `.claude-plugin/plugin.json` 示例
```json
{
  "name": "repo-radar",
  "version": "0.1.0",
  "description": "统计仓库 TODO 与近期提交，并提供可视化侧栏面板",
  "main": "index.mjs"
}
```

## 3. Tier-1 代码插件 (`index.mjs`) 完整 SDK 契约

入口模块必须默认导出 `export default function (jcode) { ... }`（纯标准库 `node:fs`、`node:path`、`node:child_process` 即可直接使用，无需 `npm install`）：

```js
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

/** @param {import("./jcode").Jcode} jcode */
export default function (jcode) {
  // 1) 注册斜杠命令：/<name> 及 /<plugin>:<name>
  // 返回字符串 => 作为用户提示词发给 Agent；返回 { message: "..." } => 直接在聊天流展示结果，不消耗模型轮次！
  jcode.command(
    "radar",
    { description: "快速统计当前工作目录提交情况", argumentHint: "[days]" },
    (args, ctx) => {
      const days = Number.parseInt(args.trim(), 10) || 7;
      try {
        const out = execFileSync("git", ["-C", ctx.cwd, "log", `--since=${days}.days`, "--oneline"], { encoding: "utf8" });
        const count = out.trim() ? out.trim().split("\n").length : 0;
        return { message: `### 最近 ${days} 天共有 ${count} 次提交\n\n\`\`\`text\n${out.slice(0, 2000) || "(无提交)"}\n\`\`\`` };
      } catch (err) {
        return { error: `读取 Git 记录失败：${err.message}` };
      }
    }
  );

  // 2) 给 Agent 自己注册新工具（自动变为 MCP 工具，新会话可直接调用）
  jcode.tool(
    "count_todos",
    {
      description: "扫描指定文本或文件中的 TODO/FIXME 数量",
      parameters: {
        type: "object",
        properties: { text: { type: "string", description: "要扫描的文本" } },
        required: ["text"],
      },
    },
    ({ text }) => {
      const matches = (text.match(/\b(TODO|FIXME|HACK)\b/g) || []).length;
      return JSON.stringify({ todo_count: matches });
    }
  );

  // 3) 生命周期钩子：pre_tool | pre_tool_transform | post_tool | post_tool_feedback | turn_start | turn_end | session_start | session_end
  jcode.on("pre_tool", (event) => {
    if (event.tool === "bash" && /\brm\s+-rf\s+\/(\s|$)/.test(event.input?.command ?? "")) {
      return { block: "已由安全插件拦截根目录删除指令。" };
    }
  });

  // 4) 桌面端可视化面板（默认在左侧边栏生成胶囊入口，点击打开独立交互面板）
  jcode.panel("dashboard", {
    title: "仓库看板",
    state: { filter: "", count: 0 },
    render(state, ui) {
      return ui.stack([
        ui.heading("仓库快捷看板"),
        ui.caption(`插件目录：${jcode.root}`),
        ui.row([
          ui.input("filter", { placeholder: "输入过滤关键词…" }),
          ui.button("刷新计数", "refresh", {}, { variant: "primary" }),
        ], { gap: "sm" }),
        ui.card([
          ui.keyValue({ "当前过滤词": state.filter || "(全部)", "刷新次数": state.count }),
        ], "运行状态"),
      ], { gap: "md", padding: "md" });
    },
    actions: {
      refresh(_args, state, { toast }) {
        toast("看板已刷新", "success");
        return { ...state, count: (state.count || 0) + 1 };
      },
    },
  });
}
```

### `ui.*` 可用面板组件速查
- 布局：`ui.stack(children, { direction, gap, padding })`、`ui.row(children, { gap })`、`ui.card(children, title?)`、`ui.scroll(children, maxHeight?)`、`ui.divider()`、`ui.spacer()`
- 文本：`ui.heading(text)`、`ui.text(text, { style, tone })`、`ui.caption(text)`、`ui.markdown(md)`、`ui.code(code, lang?)`
- 控件：`ui.button(label, actionName, args?, { variant: "primary"|"secondary"|"compact"|"danger", disabled? })`、`ui.chip(label, actionName?, args?)`、`ui.input(stateKey, { placeholder?, multiline?, onSubmit? })`、`ui.toggle(label, stateKey, onChange?)`、`ui.select(stateKey, options, onChange?)`
- 数据展示：`ui.keyValue(record)`、`ui.table(columns, rows)`、`ui.list(items)`、`ui.item(title, { subtitle?, meta?, badges?, action?, args? })`、`ui.progress(0..1, label?)`、`ui.empty(title, detail?)`、`ui.error(msg)`

## 4. 开发完成后的强制自检与启用闭环

每次创建或修改插件后，**必须**完成以下自检：
1. **检查 JSON 与 ESM 语法**：对 `index.mjs` 运行 `node --check <path/to/index.mjs>` 确保无语法错误；检查 `plugin.json` 格式合法。
2. **启用并同步插件**：
   - 确保 `~/.jcode/plugins/.state.json` 的 `"enabled"` 对象中将该插件名设为 `true`（默认新建本地插件若未显式禁用也会启用，写入 `"enabled": { "<name>": true }` 最稳妥）；
   - 若同目录存在 `jcode-plugins`（或 `jcode-plugins.exe`），运行 `jcode-plugins sync` 与 `jcode-plugins info <name>` 核验注册的 commands / tools / hooks / panels 数量且无 `warnings`。
3. **清晰告知用户如何使用**：列出已生成的目录、新增的 `/命令` 名称、MCP 工具或侧栏面板名称（Jcode Desktop 会在你完成本轮回复后自动热重载命令与面板！）。
"#;

/// Ensure `~/.jcode/skills/jcode-plugin-creator/SKILL.md` (and companion
/// `jcode.d.ts`) are present and up to date unless disabled by the user.
pub fn ensure(layout: &Layout) -> anyhow::Result<PathBuf> {
    let dir = layout.skills_dir().join(SKILL_NAME);
    if dir.join(".disabled").exists() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir)?;
    let skill_path = dir.join("SKILL.md");
    if std::fs::read_to_string(&skill_path).ok().as_deref() != Some(SKILL_MARKDOWN) {
        write_atomic(&skill_path, SKILL_MARKDOWN.as_bytes())?;
    }
    let dts_path = dir.join("jcode.d.ts");
    let dts = include_str!("jcode.d.ts");
    if std::fs::read_to_string(&dts_path).ok().as_deref() != Some(dts) {
        write_atomic(&dts_path, dts.as_bytes())?;
    }
    Ok(dir)
}

/// Build a self-contained prompt instructing the Jcode Agent to create or
/// update a plugin inside `plugins_dir`, validate it, and enable it.
pub fn build_authoring_prompt(
    plugins_dir: &Path,
    dispatcher: Option<&Path>,
    existing_plugin: Option<(&str, &Path)>,
    preferred_name: Option<&str>,
    mode_hint: &str,
    requirement: &str,
) -> String {
    let plugins_dir_str = plugins_dir.display();
    let sync_cmd = dispatcher
        .map(|exe| format!("\"{}\" sync", exe.display()))
        .unwrap_or_else(|| "jcode-plugins sync".to_string());

    let target_section = if let Some((name, root)) = existing_plugin {
        format!(
            "## 目标模式：修改并升级已有插件 `{name}`\n- 插件目录：`{}`\n- 请先读取该目录下的现有文件（如 `.claude-plugin/plugin.json`、`index.mjs`、`commands/`、`skills/`、`hooks/`），在保留原有正常功能的基础上完成改造升级。",
            root.display()
        )
    } else {
        let naming = preferred_name
            .map(crate::manifest::sanitize_name)
            .filter(|n| !n.is_empty())
            .map(|n| format!("请使用插件目录名：`{n}`（路径 `{plugins_dir_str}/{n}`）"))
            .unwrap_or_else(|| {
                format!("请根据功能起一个简洁的 kebab-case 英文插件名，创建在 `{plugins_dir_str}/<plugin-name>` 下")
            });
        format!("## 目标模式：新建并启用 Jcode 插件\n- 安装根目录：`{plugins_dir_str}`\n- 命名要求：{naming}\n- 架构偏好：{mode_hint}")
    };

    format!(
        "请为当前 Jcode Desktop 编写并启用一个本地插件。\n\n\
         {target_section}\n\n\
         ## 用户需求\n\
         {requirement}\n\n\
         ## Jcode 插件规范要点\n\
         1. 元数据位于 `<插件目录>/.claude-plugin/plugin.json`（字段：`name`, `version`, `description`；若为 Node ESM 代码插件则加 `\"main\": \"index.mjs\"` 并附带同目录 `jcode.d.ts`）。\n\
         2. **ESM 代码插件 (`index.mjs`)**：默认导出 `export default function (jcode) {{ ... }}`，支持：\n\
            - `jcode.command(name, {{ description, argumentHint }}, async (args, ctx) => string | {{ message, prompt, error }})`（返回字符串发给 AI，返回 `{{ message }}` 直接回显聊天流）\n\
            - `jcode.tool(name, {{ description, parameters }}, async (args, ctx) => string | object)`\n\
            - `jcode.on(event, async (e) => ...)`（`pre_tool` 返回 `{{ block: \"原因\" }}` 可拦截；`post_tool_feedback` 返回附加反馈）\n\
            - `jcode.panel(id, {{ title, state, render(state, ui), actions }})`（组件含 `ui.stack/row/card/scroll/heading/text/caption/markdown/code/button/chip/input/toggle/select/keyValue/table/list/item/progress/empty/error`）\n\
            - 仅使用 Node 内置模块（`node:fs`, `node:path`, `node:child_process` 等），零外部 npm 依赖。\n\
         3. **声明式插件**：可使用 `commands/<cmd>.md`（Frontmatter 含 `description`、`argument-hint`，正文用 `$ARGUMENTS`）、`skills/<id>/SKILL.md`、`hooks/hooks.json`、`.mcp.json`。\n\
         4. **自检与启用**：写完文件后请运行 `node --check` 校验 JS 语法，并运行 `{sync_cmd}` 验证零报错且已启用；最后汇总插件路径与使用方式（如新增的 `/命令` 或侧栏面板入口）。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensures_builtin_plugin_creator_skill_and_respects_disabled_flag() {
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::at(temp.path());
        let dir = ensure(&layout).unwrap();
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("jcode.d.ts").is_file());

        let prompt = build_authoring_prompt(
            &layout.plugins_dir,
            None,
            None,
            Some("My Helper"),
            "ESM 代码插件",
            "统计 Git 提交历史",
        );
        assert!(prompt.contains("my-helper"));
        assert!(prompt.contains("统计 Git 提交历史"));
    }
}
