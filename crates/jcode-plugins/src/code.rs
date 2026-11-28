//! Code plugins: a JavaScript module (`"main"` in plugin.json) that registers
//! commands, tools, event handlers and panels at runtime, like pi extensions.
//!
//! Every entry point runs the embedded Node host (`host.mjs`):
//! - commands: `node host.mjs command <main> <name> <args>`, on demand;
//! - tools: an MCP server entry (`node host.mjs mcp <main>`) in mcp.json;
//! - events: the hook dispatcher runs `node host.mjs hook <main> <event>`;
//! - panels: a Desktop applet (`node host.mjs applet <main> <id>`).
//!
//! `sync` records what each plugin registered in `.runtime/<name>.json`, so
//! Desktop and the hook dispatcher never start Node just to find out.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::manifest::Plugin;
use crate::paths::Layout;

pub const HOST: &str = include_str!("host.mjs");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CodeCommand {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub argument_hint: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub id: String,
    #[serde(default)]
    pub title: String,
}

/// What a code plugin registered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Description {
    #[serde(default)]
    pub commands: Vec<CodeCommand>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub panels: Vec<Panel>,
}

/// What a code command returned: a prompt to send, or a message to show.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct CommandReply {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn runtime_dir(layout: &Layout) -> PathBuf {
    layout.plugins_dir.join(".runtime")
}

pub fn host_path(layout: &Layout) -> PathBuf {
    runtime_dir(layout).join("host.mjs")
}

pub fn description_path(layout: &Layout, plugin: &str) -> PathBuf {
    runtime_dir(layout).join(format!("{plugin}.json"))
}

/// Node: `$JCODE_NODE`, else `node` on PATH.
pub fn node() -> Option<PathBuf> {
    if let Some(node) = std::env::var_os("JCODE_NODE").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(node));
    }
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Write the embedded host when missing or outdated.
pub fn ensure_host(layout: &Layout) -> anyhow::Result<PathBuf> {
    let path = host_path(layout);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(HOST) {
        crate::sync::write_atomic(&path, HOST.as_bytes())?;
    }
    Ok(path)
}

/// The argv for one host mode.
pub fn argv(node: &Path, layout: &Layout, plugin: &Plugin, mode: &str, extra: &[&str]) -> Option<Vec<String>> {
    let main = plugin.main_script()?;
    let mut argv = vec![
        node.to_string_lossy().into_owned(),
        host_path(layout).to_string_lossy().into_owned(),
        mode.to_string(),
        main.to_string_lossy().into_owned(),
    ];
    argv.extend(extra.iter().map(|arg| arg.to_string()));
    Some(argv)
}

/// Run a host mode and collect stdout, killing it after `timeout`.
pub fn run(argv: &[String], cwd: Option<&Path>, timeout: Duration) -> anyhow::Result<String> {
    let mut command = Command::new(&argv[0]);
    crate::paths::no_console_window(&mut command);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd.filter(|dir| dir.is_dir()) {
        command.current_dir(cwd);
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = stdout {
            let _ = std::io::Read::read_to_string(&mut pipe, &mut text);
        }
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = stderr {
            let _ = std::io::Read::read_to_string(&mut pipe, &mut text);
        }
        text
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    anyhow::ensure!(status.success(), "{}", stderr.trim());
    Ok(stdout)
}

/// Load the plugin in Node and record its registrations.
pub fn describe(layout: &Layout, plugin: &Plugin, node: &Path) -> anyhow::Result<Description> {
    let argv = argv(node, layout, plugin, "describe", &[])
        .ok_or_else(|| anyhow::anyhow!("{} has no main script", plugin.name))?;
    let stdout = run(&argv, Some(&plugin.root), Duration::from_secs(15))
        .map_err(|error| anyhow::anyhow!("{}: {error:#}", plugin.name))?;
    let description: Description = serde_json::from_str(stdout.trim())
        .map_err(|error| anyhow::anyhow!("{}: bad describe output: {error}", plugin.name))?;
    crate::sync::write_atomic(
        &description_path(layout, &plugin.name),
        serde_json::to_string_pretty(&description)?.as_bytes(),
    )?;
    Ok(description)
}

/// The registrations recorded by the last sync.
pub fn cached(layout: &Layout, plugin: &str) -> Option<Description> {
    let text = std::fs::read_to_string(description_path(layout, plugin)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Run a code command in `cwd` (the session's working directory).
pub fn run_command(
    layout: &Layout,
    plugin: &Plugin,
    name: &str,
    args: &str,
    cwd: Option<&Path>,
) -> anyhow::Result<CommandReply> {
    let node = node().ok_or_else(|| anyhow::anyhow!("Node.js was not found on PATH (set JCODE_NODE)"))?;
    ensure_host(layout)?;
    let argv = argv(&node, layout, plugin, "command", &[name, args])
        .ok_or_else(|| anyhow::anyhow!("{} has no main script", plugin.name))?;
    let stdout = run(&argv, cwd.or(Some(&plugin.root)), Duration::from_secs(120))?;
    Ok(serde_json::from_str(stdout.trim())?)
}

/// Hook entries that run enabled code plugins' handlers for `event`.
pub fn hook_entries(layout: &Layout, plugins: &[Plugin], event: &str) -> Vec<crate::hooks::HookEntry> {
    let Some(node) = node() else {
        return Vec::new();
    };
    plugins
        .iter()
        .filter(|plugin| plugin.enabled)
        .filter(|plugin| {
            plugin
                .code
                .as_ref()
                .is_some_and(|code| code.events.iter().any(|e| e == event))
        })
        .filter_map(|plugin| {
            Some(crate::hooks::HookEntry {
                plugin: plugin.name.clone(),
                root: plugin.root.clone(),
                flavor: crate::hooks::Flavor::Native,
                matcher: None,
                command: String::new(),
                timeout: None,
                argv: Some(argv(&node, layout, plugin, "hook", &[event])?),
            })
        })
        .collect()
}

/// Applet folder generated for a plugin's panels.
pub fn applet_dir(layout: &Layout, plugin: &str) -> PathBuf {
    runtime_dir(layout).join("applets").join(format!("plugin-{plugin}"))
}

pub fn applets_root(layout: &Layout) -> PathBuf {
    runtime_dir(layout).join("applets")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, Write as _};

    /// End to end through real Node; skipped where Node is not installed.
    #[test]
    fn code_plugin_registers_commands_tools_hooks_and_panels() {
        let Some(node) = node() else {
            eprintln!("skipping: Node.js not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::at(temp.path());
        crate::scaffold::create_code(&layout.plugins_dir, "demo").unwrap();
        let options = crate::SyncOptions {
            dispatcher: Some(PathBuf::from("/opt/jcode-plugins")),
        };
        let report = crate::sync(&layout, &options);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.hook_events, ["pre_tool"]);
        assert_eq!(report.mcp_servers, 1);

        let (plugins, _) = crate::discover(&layout, &crate::State::load(&layout));
        let plugin = &plugins[0];
        let code = plugin.code.as_ref().expect("description cached");
        assert_eq!(code.commands[0].name, "standup");
        assert_eq!(code.tools, ["word_count"]);
        assert_eq!(code.panels[0].id, "counter");
        assert!(applet_dir(&layout, "demo").join("applet.json").is_file());

        let catalog = crate::Catalog::build(&plugins, |_| false);
        let (command, args) = catalog.find("/standup 3").unwrap();
        assert!(command.code);
        let reply = run_command(&layout, plugin, &command.name, args, None).unwrap();
        assert!(reply.prompt.unwrap().contains("last 3 day(s)"));

        let entries = hook_entries(&layout, &plugins, "pre_tool");
        let invocation = |command: &str| crate::hooks::Invocation {
            event: "pre_tool".into(),
            tool_name: "bash".into(),
            stdin: serde_json::to_vec(&serde_json::json!({"command": command})).unwrap(),
            ..Default::default()
        };
        let blocked = crate::hooks::dispatch(&invocation("git push --force"), &entries);
        assert_eq!(blocked.code, 2);
        assert!(blocked.stderr.contains("explicit approval"));
        assert_eq!(crate::hooks::dispatch(&invocation("git status"), &entries).code, 0);

        // The MCP server answers initialize, tools/list and tools/call.
        let argv = argv(&node, &layout, plugin, "mcp", &[]).unwrap();
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut ask = |request: serde_json::Value| {
            writeln!(stdin, "{request}").unwrap();
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            serde_json::from_str::<serde_json::Value>(&line).unwrap()
        };
        let init = ask(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}));
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        let list = ask(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
        assert_eq!(list["result"]["tools"][0]["name"], "word_count");
        let call = ask(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "word_count", "arguments": {"text": "one two three"}}}));
        assert_eq!(call["result"]["content"][0]["text"], "3");
        let _ = child.kill();
    }
}
