//! `jcode-plugins new <name>`: a working starter plugin to edit.
use std::path::{Path, PathBuf};

use crate::manifest::sanitize_name;

pub fn create(plugins_dir: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let name = sanitize_name(name);
    anyhow::ensure!(!name.is_empty(), "plugin names use letters, digits, '-', '_' and '.'");
    let root = plugins_dir.join(&name);
    anyhow::ensure!(!root.exists(), "{} already exists", root.display());
    let files = [
        (
            ".claude-plugin/plugin.json".to_string(),
            serde_json::to_string_pretty(&serde_json::json!({
                "name": name,
                "version": "0.1.0",
                "description": "Describe what this plugin adds",
            }))?,
        ),
        (
            "commands/hello.md".into(),
            "---\ndescription: Say hello and summarize the project\nargument-hint: [topic]\n---\nGreet me briefly, then summarize this project in three bullet points.\nFocus on: $ARGUMENTS\n".into(),
        ),
        (
            format!("skills/{name}-notes/SKILL.md"),
            format!(
                "---\nname: {name}-notes\ndescription: Example skill from the {name} plugin. Use when the user asks for project notes.\n---\n\n# Project notes\n\nKeep notes short and factual. Prefer bullet points.\n"
            ),
        ),
        (
            "hooks/hooks.json".into(),
            "{\n  \"hooks\": {}\n}\n".into(),
        ),
        (
            "README.md".into(),
            format!(
                "# {name}\n\nA Jcode plugin.\n\n- `commands/*.md`: slash commands (`/hello`, also `/{name}:hello`)\n- `skills/*/SKILL.md`: skills\n- `hooks/hooks.json`: hooks, Claude Code format (`PreToolUse`, `PostToolUse`, `Stop`, ...) or jcode events (`pre_tool`, `turn_end`, ...)\n- `.mcp.json`: MCP servers (`{{\"mcpServers\": {{...}}}}`)\n- `applets/<id>/applet.json`: Jcode Desktop panels\n\nRun `jcode-plugins sync` (Desktop does this on start) after editing hooks, skills or MCP servers.\n"
            ),
        ),
    ];
    for (relative, text) in files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("files have parents"))?;
        std::fs::write(path, text)?;
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Plugin, Source};

    #[test]
    fn scaffold_is_a_loadable_plugin_with_a_command_and_skill() {
        let temp = tempfile::tempdir().unwrap();
        let root = create(temp.path(), "My Tools").unwrap();
        let mut plugin = Plugin::load(&root, Source::Jcode).unwrap();
        plugin.enabled = true;
        assert_eq!(plugin.name, "my-tools");
        assert_eq!(plugin.skills().len(), 1);
        let catalog = crate::Catalog::build([&plugin], |_| false);
        assert!(catalog.expand("/hello tests").unwrap().ends_with("Focus on: tests"));
        assert!(create(temp.path(), "my-tools").is_err());
    }
}
