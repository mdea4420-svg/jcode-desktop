//! Slash commands: Markdown prompt templates, as in Claude Code.
//!
//! `commands/review.md` becomes `/review` (and always `/<plugin>:review`).
//! Subfolders namespace the name: `commands/git/commit.md` is `/git:commit`.
//! Optional front matter sets `description` and `argument-hint`. The body may
//! use `$ARGUMENTS`, `$1`..`$9` and `${CLAUDE_PLUGIN_ROOT}`. Without any
//! placeholder, the arguments are appended after the template.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::manifest::Plugin;

#[derive(Clone, Debug, PartialEq)]
pub struct PluginCommand {
    pub plugin: String,
    /// Name without the slash, `review` or `git:commit`.
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub body: String,
    pub root: PathBuf,
}

/// Split optional `---` front matter into simple `key: value` pairs.
pub fn front_matter(text: &str) -> (BTreeMap<String, String>, &str) {
    let mut fields = BTreeMap::new();
    let Some(rest) = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))
    else {
        return (fields, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let trimmed = line.trim();
        if trimmed == "---" {
            return (fields, &rest[offset..]);
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            fields.insert(key.trim().to_ascii_lowercase(), value.to_string());
        }
    }
    (BTreeMap::new(), text)
}

impl PluginCommand {
    pub fn qualified(&self) -> String {
        format!("{}:{}", self.plugin, self.name)
    }

    /// The prompt sent to the agent for `/<name> <args>`.
    pub fn expand(&self, args: &str) -> String {
        let args = args.trim();
        let root = self.root.to_string_lossy();
        let mut prompt = self
            .body
            .replace("${CLAUDE_PLUGIN_ROOT}", &root)
            .replace("${JCODE_PLUGIN_ROOT}", &root);
        let has_arguments = prompt.contains("$ARGUMENTS");
        let has_positional = (1..=9).any(|n| prompt.contains(&format!("${n}")));
        prompt = prompt.replace("$ARGUMENTS", args);
        if has_positional {
            let words = shlex::split(args)
                .unwrap_or_else(|| args.split_whitespace().map(str::to_string).collect());
            for n in (1..=9).rev() {
                let value = words.get(n - 1).map(String::as_str).unwrap_or("");
                prompt = prompt.replace(&format!("${n}"), value);
            }
        }
        let mut prompt = prompt.trim().to_string();
        if !has_arguments && !has_positional && !args.is_empty() {
            prompt.push_str("\n\nARGUMENTS: ");
            prompt.push_str(args);
        }
        prompt
    }
}

fn collect(dir: &Path, prefix: &str, plugin: &Plugin, out: &mut Vec<PluginCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if stem.is_empty() || stem.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(&path, &format!("{prefix}{stem}:"), plugin, out);
            continue;
        }
        if path.extension().is_none_or(|ext| !ext.eq_ignore_ascii_case("md")) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (fields, body) = front_matter(&text);
        let description = fields.get("description").cloned().or_else(|| {
            body.lines()
                .map(|line| line.trim().trim_start_matches('#').trim())
                .find(|line| !line.is_empty())
                .map(|line| line.chars().take(80).collect())
        });
        out.push(PluginCommand {
            plugin: plugin.name.clone(),
            name: format!("{prefix}{stem}").to_ascii_lowercase().replace(' ', "-"),
            description,
            argument_hint: fields.get("argument-hint").cloned(),
            body: body.to_string(),
            root: plugin.root.clone(),
        });
    }
}

pub fn load(plugin: &Plugin) -> Vec<PluginCommand> {
    let mut commands = Vec::new();
    for dir in plugin.command_dirs() {
        collect(&dir, "", plugin, &mut commands);
    }
    commands
}

/// Every enabled plugin command under its slash names.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    commands: Vec<PluginCommand>,
    /// Slash name (`/review`, `/tools:review`) to command index.
    names: BTreeMap<String, usize>,
}

impl Catalog {
    /// `reserved` names (built-in commands) are never shadowed: such a command
    /// stays reachable as `/<plugin>:<name>`. An ambiguous short name is
    /// dropped the same way.
    pub fn build<'a>(
        plugins: impl IntoIterator<Item = &'a Plugin>,
        reserved: impl Fn(&str) -> bool,
    ) -> Self {
        let mut catalog = Catalog::default();
        let mut short: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for plugin in plugins.into_iter().filter(|plugin| plugin.enabled) {
            for command in load(plugin) {
                let index = catalog.commands.len();
                catalog
                    .names
                    .insert(format!("/{}", command.qualified()), index);
                short.entry(format!("/{}", command.name)).or_default().push(index);
                catalog.commands.push(command);
            }
        }
        for (name, indices) in short {
            if indices.len() == 1 && !reserved(&name) && !catalog.names.contains_key(&name) {
                catalog.names.insert(name, indices[0]);
            }
        }
        catalog
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn commands(&self) -> &[PluginCommand] {
        &self.commands
    }

    /// `(slash name, help)` for the composer menu, short names first.
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = self
            .names
            .iter()
            .map(|(name, &index)| {
                let command = &self.commands[index];
                let mut help = command
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("{} plugin command", command.plugin));
                if let Some(hint) = &command.argument_hint {
                    help = format!("{help} {hint}");
                }
                (name.clone(), format!("{help} ({})", command.plugin))
            })
            .collect();
        entries.sort_by_key(|(name, _)| (name.contains(':'), name.clone()));
        entries
    }

    /// The command and its arguments for a composer line such as `/review src`.
    pub fn find<'a>(&self, input: &'a str) -> Option<(&PluginCommand, &'a str)> {
        let input = input.trim();
        if !input.starts_with('/') {
            return None;
        }
        let (name, args) = input
            .split_once(char::is_whitespace)
            .map_or((input, ""), |(name, args)| (name, args));
        let index = self.names.get(&name.to_ascii_lowercase())?;
        Some((&self.commands[*index], args))
    }

    /// The prompt for a composer line, if it names a plugin command.
    pub fn expand(&self, input: &str) -> Option<String> {
        self.find(input).map(|(command, args)| command.expand(args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Source;

    fn plugin(root: &Path, name: &str, files: &[(&str, &str)]) -> Plugin {
        for (path, text) in files {
            let path = root.join(name).join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let mut plugin = Plugin::load(&root.join(name), Source::Jcode).unwrap();
        plugin.enabled = true;
        plugin
    }

    #[test]
    fn templates_expand_arguments_and_positionals() {
        let command = PluginCommand {
            plugin: "p".into(),
            name: "fix".into(),
            description: None,
            argument_hint: None,
            body: "Fix issue $1 in $2.\nAll: $ARGUMENTS\nRoot: ${CLAUDE_PLUGIN_ROOT}".into(),
            root: PathBuf::from("/plug"),
        };
        assert_eq!(
            command.expand("42 \"src/a b.rs\""),
            "Fix issue 42 in src/a b.rs.\nAll: 42 \"src/a b.rs\"\nRoot: /plug"
        );
        let plain = PluginCommand {
            body: "Review the diff.".into(),
            ..command
        };
        assert_eq!(plain.expand(""), "Review the diff.");
        assert_eq!(plain.expand("carefully"), "Review the diff.\n\nARGUMENTS: carefully");
    }

    #[test]
    fn catalog_namespaces_and_never_shadows_builtins() {
        let temp = tempfile::tempdir().unwrap();
        let tools = plugin(
            temp.path(),
            "tools",
            &[
                ("commands/review.md", "---\ndescription: Review code\nargument-hint: [path]\n---\nReview $ARGUMENTS"),
                ("commands/model.md", "Pick a model"),
                ("commands/git/commit.md", "# Commit staged work\nCommit."),
            ],
        );
        let other = plugin(temp.path(), "other", &[("commands/review.md", "Other review")]);
        let catalog = Catalog::build([&tools, &other], |name| name == "/model");

        assert_eq!(catalog.expand("/tools:review src"), Some("Review src".into()));
        assert_eq!(catalog.expand("/other:review"), Some("Other review".into()));
        // Ambiguous and reserved short names stay namespaced only.
        assert!(catalog.find("/review").is_none());
        assert!(catalog.find("/model").is_none());
        assert_eq!(catalog.expand("/tools:model"), Some("Pick a model".into()));
        assert_eq!(catalog.expand("/git:commit"), Some("# Commit staged work\nCommit.".into()));

        let entries = catalog.entries();
        assert_eq!(entries[0].0, "/git:commit");
        let review = entries.iter().find(|(name, _)| name == "/tools:review").unwrap();
        assert_eq!(review.1, "Review code [path] (tools)");
    }
}
