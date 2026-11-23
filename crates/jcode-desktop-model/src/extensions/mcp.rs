//! MCP server listing and editing for the Extensions overlay.
//!
//! Mirrors the merge order of the runtime's `McpConfig::load_for_dir`
//! (`jcode-base/src/mcp/protocol.rs`) without calling it: that loader runs a
//! one-time Codex import as a side effect, which a management UI must not
//! trigger. Sources, in override order (later wins at runtime):
//!
//! 1. `~/.jcode/mcp.json` — jcode global, editable.
//! 2. `~/.claude.json` top-level `mcpServers` — read-only (Claude Code).
//! 3. `~/.claude.json` `projects[<dir>].mcpServers` — read-only.
//! 4. `~/.claude/mcp.json` — legacy Claude file, read-only.
//! 5. `<project>/.jcode/mcp.json` — editable.
//! 6. `<project>/.mcp.json` — editable.
//! 7. `<project>/.claude/mcp.json` — read-only.
//!
//! Servers recorded in `~/.jcode/plugins/.state.json` (`managed.mcp`) belong
//! to fork plugins and are read-only here: `/plugins` manages their lifecycle.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::ExtensionPaths;

/// Which config file an MCP server entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum McpSource {
    /// `~/.jcode/mcp.json`.
    JcodeGlobal,
    /// `<project>/.jcode/mcp.json`.
    ProjectJcode,
    /// `<project>/.mcp.json` (Claude Code project config).
    ProjectMcp,
    /// `<project>/.claude/mcp.json`.
    ProjectClaude,
    /// `~/.claude.json` top-level `mcpServers`, or the legacy
    /// `~/.claude/mcp.json` file (same read-only Claude Code source).
    ClaudeUser,
    /// `~/.claude.json` `projects[<dir>].mcpServers`.
    ClaudeProject,
}

impl McpSource {
    /// Sources the overlay may edit in place.
    pub fn editable(self) -> bool {
        matches!(
            self,
            McpSource::JcodeGlobal | McpSource::ProjectJcode | McpSource::ProjectMcp
        )
    }

    /// Source group header in the overlay.
    pub fn label(self) -> &'static str {
        match self {
            McpSource::JcodeGlobal => "jcode · ~/.jcode/mcp.json",
            McpSource::ProjectJcode => "Project · .jcode/mcp.json",
            McpSource::ProjectMcp => "Project · .mcp.json",
            McpSource::ProjectClaude => "Project · .claude/mcp.json",
            McpSource::ClaudeUser => "Claude Code · user",
            McpSource::ClaudeProject => "Claude Code · project",
        }
    }
}

/// Normalized transport for display and the edit form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpTransport {
    #[default]
    Stdio,
    /// "http" / "streamable-http" — the modern remote transport.
    Http,
    /// Legacy HTTP+SSE.
    Sse,
}

impl McpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            McpTransport::Stdio => "stdio",
            McpTransport::Http => "http",
            McpTransport::Sse => "sse",
        }
    }
}

/// One configured MCP server as it appears in a config file.
#[derive(Clone, Debug, PartialEq)]
pub struct McpEntry {
    pub name: String,
    pub source: McpSource,
    /// File the entry lives in (edits target it directly).
    pub path: PathBuf,
    pub transport: McpTransport,
    pub command: String,
    pub args: Vec<String>,
    /// Raw values — secrets. The UI masks them everywhere but the edit form.
    pub env: Vec<(String, String)>,
    pub url: Option<String>,
    /// Raw values — secrets, same masking rule as `env`.
    pub headers: Vec<(String, String)>,
    pub enabled: bool,
    /// Writable source AND not plugin-managed.
    pub editable: bool,
    /// Owning fork plugin when the server is plugin-managed.
    pub plugin: Option<String>,
}

/// Everything `list_mcp` found: entries plus per-file parse notices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpList {
    pub entries: Vec<McpEntry>,
    /// Human-readable problems (e.g. an invalid JSON file) — never fatal.
    pub notices: Vec<String>,
}

/// A server being added or edited in the form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpDraft {
    pub transport: McpTransport,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// Parsed result of `jcode mcp test <name>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpTestResult {
    pub ok: bool,
    pub server: Option<String>,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

/// List every configured MCP server across all sources.
pub fn list_mcp(paths: &ExtensionPaths) -> McpList {
    let owned = plugin_owned_servers(&paths.jcode_home);
    let mut list = McpList::default();

    // jcode global.
    read_servers_file(
        &paths.jcode_home.join("mcp.json"),
        McpSource::JcodeGlobal,
        &owned,
        &mut list,
    );

    // Claude Code user config (~/.claude.json): global + per-project sets.
    let claude_json = paths.user_home.join(".claude.json");
    if claude_json.is_file() {
        match std::fs::read_to_string(&claude_json)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            Some(doc) => {
                if let Some(servers) = doc.get("mcpServers").and_then(Value::as_object) {
                    collect_entries(
                        servers,
                        McpSource::ClaudeUser,
                        &claude_json,
                        &owned,
                        &mut list,
                    );
                }
                if let Some(project_dir) = &paths.project_dir
                    && let Some(servers) = doc
                        .get("projects")
                        .and_then(Value::as_object)
                        .and_then(|projects| projects.get(project_dir.to_string_lossy().as_ref()))
                        .and_then(|project| project.get("mcpServers"))
                        .and_then(Value::as_object)
                {
                    collect_entries(
                        servers,
                        McpSource::ClaudeProject,
                        &claude_json,
                        &owned,
                        &mut list,
                    );
                }
            }
            None => list
                .notices
                .push(format!("{} is not valid JSON", claude_json.display())),
        }
    }

    // Legacy Claude Code global file.
    read_servers_file(
        &paths.user_home.join(".claude").join("mcp.json"),
        McpSource::ClaudeUser,
        &owned,
        &mut list,
    );

    // Project-local files.
    if let Some(project) = &paths.project_dir {
        for (relative, source) in [
            (".jcode/mcp.json", McpSource::ProjectJcode),
            (".mcp.json", McpSource::ProjectMcp),
            (".claude/mcp.json", McpSource::ProjectClaude),
        ] {
            read_servers_file(&project.join(relative), source, &owned, &mut list);
        }
    }

    list
}

/// Server names plugins manage (from `~/.jcode/plugins/.state.json`), each
/// with the owning plugin name resolved from the state's `enabled` map.
fn plugin_owned_servers(jcode_home: &Path) -> Vec<(String, String)> {
    let state_path = jcode_home.join("plugins").join(".state.json");
    let Ok(text) = std::fs::read_to_string(&state_path) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let plugin_names: Vec<String> = state
        .get("enabled")
        .and_then(Value::as_object)
        .map(|enabled| enabled.keys().cloned().collect())
        .unwrap_or_default();
    let mut owned = Vec::new();
    for name in state
        .get("managed")
        .and_then(|managed| managed.get("mcp"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        // Servers sync as `<plugin>-<server>`; the longest matching plugin
        // name wins because plugin names may themselves contain dashes.
        let plugin = plugin_names
            .iter()
            .filter(|plugin| name.starts_with(&format!("{plugin}-")))
            .max_by_key(|plugin| plugin.len())
            .cloned()
            .unwrap_or_else(|| {
                name.split('-')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            });
        owned.push((name.to_string(), plugin));
    }
    owned
}

/// Read a `{"mcpServers"|"servers": {...}}` file into entries; missing files
/// are skipped, invalid ones leave a notice.
fn read_servers_file(
    path: &Path,
    source: McpSource,
    owned: &[(String, String)],
    list: &mut McpList,
) {
    if !path.is_file() {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        list.notices.push(format!("{} is not readable", path.display()));
        return;
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
        list.notices
            .push(format!("{} is not valid JSON", path.display()));
        return;
    };
    let servers = doc
        .get("mcpServers")
        .or_else(|| doc.get("servers"))
        .and_then(Value::as_object);
    match servers {
        Some(servers) => collect_entries(servers, source, path, owned, list),
        None if doc.as_object().is_some_and(|root| root.is_empty()) => {}
        None => list.notices.push(format!(
            "{} has no `mcpServers` object",
            path.display()
        )),
    }
}

fn collect_entries(
    servers: &Map<String, Value>,
    source: McpSource,
    path: &Path,
    owned: &[(String, String)],
    list: &mut McpList,
) {
    for (name, config) in servers {
        let plugin = owned
            .iter()
            .find(|(managed, _)| managed == name)
            .map(|(_, plugin)| plugin.clone());
        list.entries.push(McpEntry {
            name: name.clone(),
            source,
            path: path.to_path_buf(),
            transport: transport_of(config),
            command: string_field(config, "command"),
            args: string_list(config.get("args")),
            env: string_pairs(config.get("env")),
            url: config
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_string),
            headers: string_pairs(config.get("headers")),
            enabled: enabled_of(config),
            editable: source.editable() && plugin.is_none(),
            plugin,
        });
    }
    list.entries
        .sort_by(|left, right| left.name.cmp(&right.name));
}

/// Transport as the runtime resolves it (protocol.rs): explicit `type`, else
/// a `url` without `command` counts as http, else stdio.
fn transport_of(config: &Value) -> McpTransport {
    match config
        .get("type")
        .and_then(Value::as_str)
        .map(|kind| kind.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("stdio") => McpTransport::Stdio,
        Some("sse") => McpTransport::Sse,
        Some("http" | "streamable-http" | "streamable_http" | "streamablehttp") => {
            McpTransport::Http
        }
        Some(_) | None => {
            if config.get("url").and_then(Value::as_str).is_some()
                && string_field(config, "command").trim().is_empty()
            {
                McpTransport::Http
            } else {
                McpTransport::Stdio
            }
        }
    }
}

/// `disabled: true` wins over `enabled`, matching the runtime config.
fn enabled_of(config: &Value) -> bool {
    if config
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    config
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn string_field(config: &Value, key: &str) -> String {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_pairs(value: Option<&Value>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        value.as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    pairs.sort();
    pairs
}

// ---------------------------------------------------------------- edits ----

// Files already backed up this session: the first write to each config file
// saves a `.bak` copy of the original, later writes leave it alone.
thread_local! {
    static BACKED_UP: std::cell::RefCell<BTreeSet<PathBuf>> = Default::default();
}

fn backup_once(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut bak = path.as_os_str().to_os_string();
    bak.push(".bak");
    let bak = PathBuf::from(bak);
    let done = BACKED_UP.with(|set| set.borrow().contains(path) || bak.exists());
    if done {
        return Ok(());
    }
    std::fs::copy(path, &bak)?;
    BACKED_UP.with(|set| set.borrow_mut().insert(path.to_path_buf()));
    Ok(())
}

/// The server map of a config document: the existing `mcpServers`/`servers`
/// key, or a fresh `mcpServers` object for new files.
fn servers_map_mut<'a>(
    doc: &'a mut Value,
    path: &Path,
) -> Result<&'a mut Map<String, Value>, String> {
    let root = doc
        .as_object_mut()
        .ok_or_else(|| format!("{} must be a JSON object", path.display()))?;
    let key = if root.contains_key("mcpServers") {
        "mcpServers"
    } else if root.contains_key("servers") {
        "servers"
    } else {
        "mcpServers"
    };
    root.entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("`{key}` in {} must be an object", path.display()))
}

fn read_doc(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_or_new_doc(path: &Path) -> Result<Value, String> {
    if path.exists() {
        read_doc(path)
    } else {
        Ok(Value::Object(Map::new()))
    }
}

/// Atomic write: temp file + rename, with the first-write `.bak` snapshot.
fn write_doc(path: &Path, doc: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    backup_once(path).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(doc).map_err(|error| error.to_string())?;
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, path).map_err(|error| error.to_string())
}

/// Enable/disable a server in its own config file. Disabling writes
/// `"disabled": true` (and drops `enabled`); enabling removes `disabled` and
/// any `enabled: false`.
pub fn set_mcp_enabled(path: &Path, name: &str, enabled: bool) -> Result<(), String> {
    let mut doc = read_doc(path)?;
    let servers = servers_map_mut(&mut doc, path)?;
    let config = servers
        .get_mut(name)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("No server `{name}` in {}", path.display()))?;
    if enabled {
        config.remove("disabled");
        if config.get("enabled") == Some(&Value::Bool(false)) {
            config.remove("enabled");
        }
    } else {
        config.insert("disabled".to_string(), Value::Bool(true));
        config.remove("enabled");
    }
    write_doc(path, &doc)
}

/// Add a server (`original_name: None`) or replace one (rename supported).
/// Only the managed fields are rewritten — unknown fields survive edits.
pub fn upsert_mcp(
    path: &Path,
    original_name: Option<&str>,
    name: &str,
    draft: &McpDraft,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Server name is required.".into());
    }
    let mut doc = read_or_new_doc(path)?;
    let servers = servers_map_mut(&mut doc, path)?;
    if Some(name) != original_name && servers.contains_key(name) {
        return Err(format!("A server named `{name}` already exists in this file."));
    }
    let mut config = match original_name {
        Some(original) if servers.contains_key(original) => servers
            .remove(original)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        _ => Map::new(),
    };
    for key in ["type", "command", "args", "env", "url", "headers"] {
        config.remove(key);
    }
    match draft.transport {
        McpTransport::Stdio => {
            config.insert("command".into(), Value::String(draft.command.clone()));
            if !draft.args.is_empty() {
                config.insert(
                    "args".into(),
                    Value::Array(
                        draft.args.iter().cloned().map(Value::String).collect(),
                    ),
                );
            }
            if !draft.env.is_empty() {
                config.insert("env".into(), pairs_value(&draft.env));
            }
        }
        McpTransport::Http | McpTransport::Sse => {
            config.insert(
                "type".into(),
                Value::String(draft.transport.as_str().to_string()),
            );
            config.insert("url".into(), Value::String(draft.url.clone()));
            if !draft.headers.is_empty() {
                config.insert("headers".into(), pairs_value(&draft.headers));
            }
        }
    }
    servers.insert(name.to_string(), Value::Object(config));
    write_doc(path, &doc)
}

fn pairs_value(pairs: &[(String, String)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect(),
    )
}

/// Remove a server from its config file.
pub fn delete_mcp(path: &Path, name: &str) -> Result<(), String> {
    let mut doc = read_doc(path)?;
    let servers = servers_map_mut(&mut doc, path)?;
    if servers.remove(name).is_none() {
        return Err(format!("No server `{name}` in {}", path.display()));
    }
    write_doc(path, &doc)
}

// --------------------------------------------------------------- paste ----

/// Parse pasted MCP JSON into `(name, draft)` rows. Accepts
/// `{"mcpServers":{…}}`, `{"<name>":{…}, …}`, or a single server object
/// (returned with an empty name — the form's name field supplies it).
pub fn parse_mcp_paste(text: &str) -> Result<Vec<(String, McpDraft)>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("Not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Expected a JSON object.".to_string())?;
    let servers = object
        .get("mcpServers")
        .or_else(|| object.get("servers"))
        .and_then(Value::as_object);
    if let Some(servers) = servers {
        return servers
            .iter()
            .map(|(name, config)| draft_from_value(config).map(|draft| (name.clone(), draft)))
            .collect();
    }
    let single = object.contains_key("command")
        || object.contains_key("url")
        || object.contains_key("type");
    if !single && !object.is_empty() && object.values().all(Value::is_object) {
        return object
            .iter()
            .map(|(name, config)| draft_from_value(config).map(|draft| (name.clone(), draft)))
            .collect();
    }
    if single {
        return draft_from_value(&value).map(|draft| vec![(String::new(), draft)]);
    }
    Err("Paste a server object, a `{name: {…}}` map, or `{\"mcpServers\":{…}}`.".into())
}

fn draft_from_value(config: &Value) -> Result<McpDraft, String> {
    if !config.is_object() {
        return Err("Each server entry must be a JSON object.".into());
    }
    Ok(McpDraft {
        transport: transport_of(config),
        command: string_field(config, "command"),
        args: string_list(config.get("args")),
        env: string_pairs(config.get("env")),
        url: string_field(config, "url"),
        headers: string_pairs(config.get("headers")),
    })
}

// ----------------------------------------------------------------- test ----

/// Parse `jcode mcp test <name>` output into a displayable result.
///
/// ```text
/// Connected to MCP server 'fs' and disconnected cleanly.
/// Server: filesystem 1.2.3
/// Tools: 2
///   read_file
///   write_file
/// ```
pub fn parse_mcp_test_output(stdout: &str, stderr: &str, ok: bool) -> McpTestResult {
    if !ok {
        let detail = [stderr, stdout]
            .iter()
            .map(|text| text.trim())
            .find(|text| !text.is_empty())
            .unwrap_or("test failed")
            .to_string();
        return McpTestResult {
            ok: false,
            server: None,
            tools: Vec::new(),
            error: Some(detail),
        };
    }
    let mut server = None;
    let mut tools = Vec::new();
    let mut in_tools = false;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(info) = trimmed.strip_prefix("Server: ") {
            server = Some(info.trim().to_string());
        } else if trimmed.starts_with("Tools: ") {
            in_tools = true;
        } else if in_tools && line.starts_with("  ") && !trimmed.is_empty() {
            tools.push(trimmed.to_string());
        } else if in_tools && !trimmed.is_empty() {
            in_tools = false;
        }
    }
    McpTestResult {
        ok: true,
        server,
        tools,
        error: None,
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
