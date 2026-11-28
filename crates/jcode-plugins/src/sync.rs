//! Apply the enabled plugins to the jcode runtime config.
//!
//! - Skills: a link per skill in `~/.jcode/skills/plugin-<plugin>-<skill>`.
//! - MCP: servers merged into `~/.jcode/mcp.json` as `<plugin>-<server>`.
//! - Hooks: one `jcode-plugins hook <event>` entry per used event in
//!   `[hooks]` of `~/.jcode/config.toml`, appended to the user's own hooks.
//!
//! Only entries recorded in the state file (or recognizably the dispatcher's)
//! are ever changed or removed. Claude Code plugins contribute MCP servers and
//! hooks; their skills are already loaded by jcode itself.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::code;
use crate::hooks::{self, EVENTS};
use crate::manifest::{Plugin, Source, discover, sanitize_name};
use crate::paths::{Layout, command_token};
use crate::state::State;
use crate::toml_edit;

#[derive(Clone, Debug, Default)]
pub struct SyncOptions {
    /// The `jcode-plugins` executable used as the hook dispatcher. Without
    /// it, hooks are not registered.
    pub dispatcher: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncReport {
    pub plugins: usize,
    pub enabled: usize,
    pub skills: usize,
    pub mcp_servers: usize,
    pub hook_events: Vec<String>,
    pub warnings: Vec<String>,
    pub changed: bool,
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("jcode-plugins.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn remove_link(path: &Path) -> std::io::Result<()> {
    // Directory links (and Windows junctions) are removed as directories;
    // this deletes the link only, never the plugin's files.
    std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
}

fn create_link(target: &Path, link: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return Ok(());
        }
        // Symlinks need Developer Mode; junctions work for every user.
        let mut command = std::process::Command::new("cmd");
        crate::paths::no_console_window(&mut command);
        let status = command
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        anyhow::ensure!(status.success(), "mklink /J failed for {}", link.display());
        Ok(())
    }
}

fn sync_skills(layout: &Layout, plugins: &[Plugin], state: &mut State, report: &mut SyncReport) {
    let skills_dir = layout.skills_dir();
    let desired: Vec<(PathBuf, PathBuf)> = plugins
        .iter()
        .filter(|plugin| plugin.enabled && plugin.source == Source::Jcode)
        .flat_map(|plugin| {
            plugin.skills().into_iter().map(|(skill, dir)| {
                let link = skills_dir.join(format!(
                    "plugin-{}-{}",
                    plugin.name,
                    sanitize_name(&skill)
                ));
                (link, dir)
            })
        })
        .collect();
    let mut managed = Vec::new();
    for old in std::mem::take(&mut state.managed.skills) {
        let keep = desired.iter().any(|(link, target)| {
            link == &old && std::fs::read_link(&old).is_ok_and(|current| same_path(&current, target))
        });
        if keep {
            managed.push(old);
        } else if is_link(&old) {
            match remove_link(&old) {
                Ok(()) => report.changed = true,
                Err(error) => {
                    report.warnings.push(format!("could not remove {}: {error}", old.display()));
                    managed.push(old);
                }
            }
        }
    }
    for (link, target) in desired {
        if managed.contains(&link) {
            continue;
        }
        if std::fs::symlink_metadata(&link).is_ok() {
            report.warnings.push(format!(
                "skipped skill link {}: something else already exists there",
                link.display()
            ));
            continue;
        }
        let _ = std::fs::create_dir_all(&skills_dir);
        match create_link(&target, &link) {
            Ok(()) => {
                managed.push(link);
                report.changed = true;
            }
            Err(error) => report.warnings.push(format!("{error:#}")),
        }
    }
    report.skills = managed.len();
    state.managed.skills = managed;
}

fn same_path(a: &Path, b: &Path) -> bool {
    let normalize = |path: &Path| {
        let text = path.to_string_lossy().replace('\\', "/");
        text.trim_start_matches("//?/").trim_end_matches('/').to_ascii_lowercase()
    };
    normalize(a) == normalize(b)
        || std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(a, b)| a == b)
}

/// Describe enabled code plugins with Node and generate their panel applets.
fn sync_code(layout: &Layout, plugins: &mut [Plugin], report: &mut SyncReport) {
    let mut applets = BTreeSet::new();
    let has_code = plugins
        .iter()
        .any(|plugin| plugin.enabled && plugin.main_script().is_some());
    let node = code::node();
    if has_code && node.is_none() {
        report
            .warnings
            .push("Node.js was not found on PATH (set JCODE_NODE); code plugins are skipped".into());
    }
    if let (true, Some(node)) = (has_code, node.as_deref()) {
        if let Err(error) = code::ensure_host(layout) {
            report.warnings.push(format!("plugin host: {error:#}"));
        }
        for plugin in plugins
            .iter_mut()
            .filter(|plugin| plugin.enabled && plugin.main_script().is_some())
        {
            match code::describe(layout, plugin, node) {
                Ok(description) => plugin.code = Some(description),
                Err(error) => report.warnings.push(format!("{error:#}")),
            }
            let has_panels = plugin.code.as_ref().is_some_and(|code| !code.panels.is_empty());
            let id = format!("plugin-{}", plugin.name);
            if let (true, Some(argv)) = (has_panels, code::argv(node, layout, plugin, "applet", &[&id])) {
                let manifest = serde_json::json!({"id": id, "command": argv, "autostart": true});
                let path = code::applet_dir(layout, &plugin.name).join("applet.json");
                let text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
                if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                    match write_atomic(&path, text.as_bytes()) {
                        Ok(()) => report.changed = true,
                        Err(error) => report.warnings.push(format!("{}: {error:#}", plugin.name)),
                    }
                }
                applets.insert(id);
            }
        }
    }
    // Generated applets of disabled or removed plugins.
    if let Ok(entries) = std::fs::read_dir(code::applets_root(layout)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("plugin-") && !applets.contains(&name) {
                let _ = std::fs::remove_dir_all(entry.path());
                report.changed = true;
            }
        }
    }
}

fn sync_mcp(layout: &Layout, plugins: &[Plugin], state: &mut State, report: &mut SyncReport) -> anyhow::Result<()> {
    let mut desired = serde_json::Map::new();
    if let Some(node) = code::node() {
        for plugin in plugins.iter().filter(|plugin| plugin.enabled) {
            let has_tools = plugin.code.as_ref().is_some_and(|code| !code.tools.is_empty());
            if let (true, Some(argv)) = (has_tools, code::argv(&node, layout, plugin, "mcp", &[])) {
                desired.insert(
                    format!("{}-tools", plugin.name),
                    serde_json::json!({"command": argv[0], "args": argv[1..]}),
                );
            }
        }
    }
    for plugin in plugins.iter().filter(|plugin| plugin.enabled) {
        match plugin.mcp_servers() {
            Ok(servers) => {
                for (name, server) in servers {
                    desired.insert(format!("{}-{}", plugin.name, sanitize_name(&name)), server);
                }
            }
            Err(error) => report.warnings.push(format!("{}: {error:#}", plugin.name)),
        }
    }
    let path = layout.mcp_json();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.trim().is_empty() && desired.is_empty() && state.managed.mcp.is_empty() {
        return Ok(());
    }
    let mut document: Value = if text.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(&text)
            .map_err(|error| anyhow::anyhow!("{} is not valid JSON: {error}", path.display()))?
    };
    let root = document
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{} must be a JSON object", path.display()))?;
    let key = if root.contains_key("servers") || !root.contains_key("mcpServers") {
        "servers"
    } else {
        "mcpServers"
    };
    let servers = root
        .entry(key)
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("`{key}` in {} must be an object", path.display()))?;
    let before = servers.clone();
    for name in &state.managed.mcp {
        if !desired.contains_key(name) {
            servers.remove(name);
        }
    }
    let mut managed = Vec::new();
    for (name, server) in desired {
        if servers.contains_key(&name) && !state.managed.mcp.contains(&name) {
            report
                .warnings
                .push(format!("MCP server `{name}` already exists in mcp.json; left as is"));
            continue;
        }
        servers.insert(name.clone(), server);
        managed.push(name);
    }
    report.mcp_servers = managed.len();
    state.managed.mcp = managed;
    if *servers != before {
        write_atomic(&path, serde_json::to_string_pretty(&document)?.as_bytes())?;
        report.changed = true;
    }
    Ok(())
}

/// The dispatcher entry for `event`, as written into config.toml.
pub fn dispatcher_entry(dispatcher: &Path, event: &str) -> String {
    format!("{} hook {event}", command_token(dispatcher))
}

fn is_dispatcher_entry(entry: &str, event: &str) -> bool {
    entry.contains("jcode-plugins") && entry.trim_end().ends_with(&format!(" hook {event}"))
}

fn sync_hooks(
    layout: &Layout,
    plugins: &[Plugin],
    options: &SyncOptions,
    report: &mut SyncReport,
) -> anyhow::Result<()> {
    let mut used = BTreeSet::new();
    for plugin in plugins.iter().filter(|plugin| plugin.enabled) {
        match hooks::plugin_hooks(plugin) {
            Ok((entries, unsupported)) => {
                used.extend(entries.iter().map(|(event, _)| *event));
                report.warnings.extend(unsupported);
            }
            Err(error) => report.warnings.push(format!("{}: {error:#}", plugin.name)),
        }
        if let Some(code) = &plugin.code {
            used.extend(
                code.events
                    .iter()
                    .filter_map(|event| EVENTS.iter().find(|known| *known == event).copied()),
            );
        }
    }
    report.hook_events = used.iter().map(|event| event.to_string()).collect();
    let path = layout.config_toml();
    let original = std::fs::read_to_string(&path).unwrap_or_default();
    let config: toml::Table = original
        .parse()
        .map_err(|error| anyhow::anyhow!("{} is not valid TOML: {error}", path.display()))?;
    let hooks_table = config.get("hooks").and_then(toml::Value::as_table);
    let mut text = original.clone();
    for event in EVENTS {
        let current = toml_edit::string_list(hooks_table.and_then(|table| table.get(event)));
        let mut next: Vec<String> = current
            .iter()
            .filter(|entry| !is_dispatcher_entry(entry, event))
            .cloned()
            .collect();
        if used.contains(event)
            && let Some(dispatcher) = &options.dispatcher
        {
            next.push(dispatcher_entry(dispatcher, event));
        }
        if next == current {
            continue;
        }
        text = toml_edit::set_value(&text, "hooks", event, &toml_edit::list_literal(&next))?;
    }
    if text != original {
        write_atomic(&path, text.as_bytes())?;
        report.changed = true;
    }
    Ok(())
}

/// Discover plugins and apply the enabled ones. Each part runs even if
/// another fails; failures are returned as warnings.
pub fn sync(layout: &Layout, options: &SyncOptions) -> SyncReport {
    let mut state = State::load(layout);
    let (mut plugins, errors) = discover(layout, &state);
    let mut report = SyncReport {
        plugins: plugins.len(),
        enabled: plugins.iter().filter(|plugin| plugin.enabled).count(),
        warnings: errors,
        ..Default::default()
    };
    sync_code(layout, &mut plugins, &mut report);
    sync_skills(layout, &plugins, &mut state, &mut report);
    if let Err(error) = sync_mcp(layout, &plugins, &mut state, &mut report) {
        report.warnings.push(format!("MCP: {error:#}"));
    }
    if let Err(error) = sync_hooks(layout, &plugins, options, &mut report) {
        report.warnings.push(format!("hooks: {error:#}"));
    }
    if layout.plugins_dir.is_dir() || !state.managed.skills.is_empty() || !state.managed.mcp.is_empty() {
        if let Err(error) = state.save(layout) {
            report.warnings.push(format!("state: {error:#}"));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn sync_registers_and_removes_only_its_own_entries() {
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::at(temp.path());
        let plugin = layout.plugins_dir.join("guard");
        write(&plugin.join("skills/triage/SKILL.md"), "---\nname: triage\n---\n");
        write(
            &plugin.join(".mcp.json"),
            r#"{"mcpServers": {"notes": {"command": "node", "args": ["${CLAUDE_PLUGIN_ROOT}/s.js"]}}}"#,
        );
        write(
            &plugin.join("hooks/hooks.json"),
            r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "x"}]}]}}"#,
        );
        write(&layout.config_toml(), "# user\n[hooks]\npre_tool = \"~/bin/policy\"\n");
        write(&layout.mcp_json(), r#"{"servers": {"mine": {"command": "a"}}}"#);
        let options = SyncOptions {
            dispatcher: Some(PathBuf::from("/opt/jcode/jcode-plugins")),
        };

        let report = sync(&layout, &options);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!((report.skills, report.mcp_servers), (1, 1));
        assert_eq!(report.hook_events, ["pre_tool"]);
        let config = std::fs::read_to_string(layout.config_toml()).unwrap();
        assert!(config.contains(
            r#"pre_tool = ["~/bin/policy", "/opt/jcode/jcode-plugins hook pre_tool"]"#
        ));
        assert!(config.starts_with("# user\n"));
        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(layout.mcp_json()).unwrap()).unwrap();
        assert!(mcp["servers"]["mine"].is_object());
        assert!(mcp["servers"]["guard-notes"]["args"][0].as_str().unwrap().ends_with("s.js"));
        let link = layout.skills_dir().join("plugin-guard-triage");
        assert!(link.join("SKILL.md").is_file());

        // A second sync is a no-op.
        assert!(!sync(&layout, &options).changed);

        // Disabling removes exactly what was added.
        let mut state = State::load(&layout);
        state.set_enabled("guard", false);
        state.save(&layout).unwrap();
        let report = sync(&layout, &options);
        assert!(report.changed);
        let config = std::fs::read_to_string(layout.config_toml()).unwrap();
        assert!(config.contains(r#"pre_tool = ["~/bin/policy"]"#));
        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(layout.mcp_json()).unwrap()).unwrap();
        assert!(mcp["servers"].get("guard-notes").is_none());
        assert!(mcp["servers"]["mine"].is_object());
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(plugin.join("skills/triage/SKILL.md").is_file(), "plugin files untouched");
    }
}
