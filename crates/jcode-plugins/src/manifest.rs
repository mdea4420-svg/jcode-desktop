use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::Deserialize;
use serde_json::Value;

use crate::paths::Layout;
use crate::state::State;

/// Where a plugin was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `~/.jcode/plugins/<name>`.
    Jcode,
    /// An installed Claude Code plugin.
    Claude,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Jcode => "jcode",
            Source::Claude => "claude",
        }
    }
}

/// One path or several, as Claude Code manifests allow.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PathList {
    One(String),
    Many(Vec<String>),
}

impl PathList {
    fn iter(&self) -> impl Iterator<Item = &str> {
        let items: Vec<&str> = match self {
            PathList::One(path) => vec![path.as_str()],
            PathList::Many(paths) => paths.iter().map(String::as_str).collect(),
        };
        items.into_iter()
    }
}

/// `.claude-plugin/plugin.json` or `plugin.json`. Every field is optional.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub author: Option<Value>,
    pub homepage: Option<String>,
    pub commands: Option<PathList>,
    pub skills: Option<PathList>,
    pub applets: Option<PathList>,
    /// A path to a hooks file, or the hooks object inline.
    pub hooks: Option<Value>,
    /// A path to an MCP config file, or the servers object inline.
    pub mcp_servers: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub root: PathBuf,
    pub source: Source,
    pub enabled: bool,
    pub manifest: Manifest,
}

/// Lowercase letters, digits, `-`, `_` and `.`; anything else becomes `-`.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    cleaned.trim_matches('-').to_string()
}

impl Plugin {
    /// Read a plugin directory. A missing manifest is allowed; the directory
    /// name is used.
    pub fn load(root: &Path, source: Source) -> anyhow::Result<Self> {
        let manifest_path = [".claude-plugin/plugin.json", "plugin.json"]
            .iter()
            .map(|relative| root.join(relative))
            .find(|path| path.is_file());
        let manifest: Manifest = match &manifest_path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", path.display()))?
            }
            None => Manifest::default(),
        };
        let fallback = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "plugin".into());
        let name = sanitize_name(manifest.name.as_deref().unwrap_or(&fallback));
        anyhow::ensure!(!name.is_empty(), "plugin at {} has no usable name", root.display());
        Ok(Self {
            name,
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            root: root.to_path_buf(),
            source,
            enabled: false,
            manifest,
        })
    }

    fn dirs(&self, list: &Option<PathList>, default: &str) -> Vec<PathBuf> {
        let mut dirs = vec![self.root.join(default)];
        if let Some(list) = list {
            dirs.extend(list.iter().map(|path| self.resolve(path)));
        }
        dirs.retain(|dir| dir.is_dir());
        dirs.dedup();
        dirs
    }

    /// A manifest path, relative to the plugin root.
    pub fn resolve(&self, path: &str) -> PathBuf {
        let expanded = self.expand_root(path);
        let path = Path::new(&expanded);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path.strip_prefix("./").unwrap_or(path))
        }
    }

    pub fn command_dirs(&self) -> Vec<PathBuf> {
        self.dirs(&self.manifest.commands, "commands")
    }

    pub fn skill_dirs(&self) -> Vec<PathBuf> {
        self.dirs(&self.manifest.skills, "skills")
    }

    pub fn applet_dirs(&self) -> Vec<PathBuf> {
        self.dirs(&self.manifest.applets, "applets")
    }

    /// `skills/<name>/SKILL.md` folders.
    pub fn skills(&self) -> Vec<(String, PathBuf)> {
        let mut skills = Vec::new();
        for dir in self.skill_dirs() {
            if dir.join("SKILL.md").is_file() {
                let name = dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                skills.push((name, dir));
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut found: Vec<_> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.join("SKILL.md").is_file())
                .map(|path| {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (name, path)
                })
                .collect();
            found.sort();
            skills.extend(found);
        }
        skills
    }

    /// The hooks document: inline, from the manifest path, or `hooks/hooks.json`.
    pub fn hooks_document(&self) -> anyhow::Result<Option<Value>> {
        match &self.manifest.hooks {
            Some(Value::String(path)) => read_json(&self.resolve(path)).map(Some),
            Some(value @ Value::Object(_)) => Ok(Some(value.clone())),
            Some(_) => anyhow::bail!("`hooks` must be a path or an object"),
            None => {
                let path = self.root.join("hooks").join("hooks.json");
                if path.is_file() {
                    read_json(&path).map(Some)
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// MCP servers with `${CLAUDE_PLUGIN_ROOT}` expanded, keyed by server name.
    pub fn mcp_servers(&self) -> anyhow::Result<serde_json::Map<String, Value>> {
        let document = match &self.manifest.mcp_servers {
            Some(Value::String(path)) => Some(read_json(&self.resolve(path))?),
            Some(value @ Value::Object(_)) => Some(value.clone()),
            Some(_) => anyhow::bail!("`mcpServers` must be a path or an object"),
            None => {
                let path = self.root.join(".mcp.json");
                path.is_file().then(|| read_json(&path)).transpose()?
            }
        };
        let Some(document) = document else {
            return Ok(Default::default());
        };
        let servers = ["mcpServers", "servers"]
            .iter()
            .find_map(|key| document.get(key).and_then(Value::as_object))
            .or_else(|| document.as_object())
            .cloned()
            .unwrap_or_default();
        Ok(servers
            .into_iter()
            .map(|(name, value)| (name, self.expand_value(value)))
            .collect())
    }

    /// Replace `${CLAUDE_PLUGIN_ROOT}` and `${JCODE_PLUGIN_ROOT}`.
    pub fn expand_root(&self, text: &str) -> String {
        let root = self.root.to_string_lossy();
        text.replace("${CLAUDE_PLUGIN_ROOT}", &root)
            .replace("${JCODE_PLUGIN_ROOT}", &root)
    }

    fn expand_value(&self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.expand_root(&text)),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|item| self.expand_value(item)).collect())
            }
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, self.expand_value(value)))
                    .collect(),
            ),
            other => other,
        }
    }
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Install paths from Claude Code's `installed_plugins.json` (version 1 maps a
/// plugin to one install, version 2 to a list).
fn claude_install_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(root.join("installed_plugins.json")) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(plugins) = document.get("plugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    plugins
        .values()
        .filter_map(|installs| match installs {
            Value::Array(list) => list.last(),
            other => Some(other),
        })
        .filter_map(|install| install.get("installPath").and_then(Value::as_str))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .collect()
}

/// Every plugin with its enabled flag. A jcode plugin hides a Claude Code
/// plugin of the same name. Unreadable plugins are returned as errors.
pub fn discover(layout: &Layout, state: &State) -> (Vec<Plugin>, Vec<String>) {
    let mut plugins: Vec<Plugin> = Vec::new();
    let mut errors = Vec::new();
    let mut roots: Vec<(PathBuf, Source)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&layout.plugins_dir) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            })
            .collect();
        dirs.sort();
        roots.extend(dirs.into_iter().map(|dir| (dir, Source::Jcode)));
    }
    if let Some(claude) = &layout.claude_plugins {
        roots.extend(
            claude_install_paths(claude)
                .into_iter()
                .map(|dir| (dir, Source::Claude)),
        );
    }
    for (root, source) in roots {
        match Plugin::load(&root, source) {
            Ok(mut plugin) => {
                if plugins.iter().any(|existing| existing.name == plugin.name) {
                    continue;
                }
                plugin.enabled = state.is_enabled(&plugin.name, source);
                plugins.push(plugin);
            }
            Err(error) => errors.push(format!("{error:#}")),
        }
    }
    (plugins, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn discovers_jcode_and_claude_plugins_with_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::at(temp.path());
        let mine = layout.plugins_dir.join("Review Tools");
        write(
            &mine.join(".claude-plugin/plugin.json"),
            r#"{"name": "review-tools", "version": "1.0.0", "description": "Review helpers"}"#,
        );
        write(&mine.join("skills/triage/SKILL.md"), "---\nname: triage\n---\n");
        write(&layout.plugins_dir.join("bare/commands/hi.md"), "Hi");
        write(&layout.plugins_dir.join(".state.json"), "{}");

        let claude = temp.path().join("cc/memo/1.0");
        write(&claude.join(".claude-plugin/plugin.json"), r#"{"name": "memo"}"#);
        write(
            &layout.claude_plugins.clone().unwrap().join("installed_plugins.json"),
            &format!(
                r#"{{"version": 2, "plugins": {{"memo@local": [{{"installPath": {}}}]}}}}"#,
                serde_json::to_string(&claude).unwrap()
            ),
        );

        let (plugins, errors) = discover(&layout, &State::default());
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<_> = plugins.iter().map(|p| (p.name.as_str(), p.enabled)).collect();
        assert_eq!(names, [("review-tools", true), ("bare", true), ("memo", false)]);
        assert_eq!(plugins[0].skills()[0].0, "triage");
        assert_eq!(plugins[1].command_dirs(), [layout.plugins_dir.join("bare/commands")]);
    }

    #[test]
    fn mcp_servers_expand_the_plugin_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("p");
        write(
            &root.join(".mcp.json"),
            r#"{"mcpServers": {"notes": {"command": "node", "args": ["${CLAUDE_PLUGIN_ROOT}/server.js"]}}}"#,
        );
        let plugin = Plugin::load(&root, Source::Jcode).unwrap();
        let servers = plugin.mcp_servers().unwrap();
        let arg = servers["notes"]["args"][0].as_str().unwrap();
        assert_eq!(arg, format!("{}/server.js", root.display()));
    }
}
