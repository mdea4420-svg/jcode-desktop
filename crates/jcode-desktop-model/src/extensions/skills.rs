//! Skill listing and editing for the Extensions overlay.
//!
//! Mirrors the runtime loader's sources (`jcode-base/src/skill.rs`): global
//! `~/.jcode/skills` and `~/.agents/skills`, Claude Code plugin installs under
//! `~/.claude/plugins`, and the project overlay `.jcode/skills`,
//! `.agents/skills`, `.claude/skills`.
//!
//! The runtime has no disable mechanism, so Desktop's own convention is used:
//! renaming `SKILL.md` to `SKILL.md.disabled` takes a directory out of every
//! loader scan (they all require `SKILL.md`).

use std::path::{Path, PathBuf};

use super::ExtensionPaths;

/// Where a skill directory lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillSource {
    /// `~/.jcode/skills`.
    Jcode,
    /// `~/.agents/skills` — the cross-tool convention shared with other
    /// agents on the machine.
    Agents,
    /// `<project>/.jcode/skills`.
    ProjectJcode,
    /// `<project>/.agents/skills`.
    ProjectAgents,
    /// `<project>/.claude/skills` — Claude Code's own files.
    ProjectClaude,
    /// A `plugin-<plugin>-<skill>` link in `~/.jcode/skills`, managed by
    /// `/plugins`.
    Plugin(String),
    /// A skill inside a Claude Code plugin install under `~/.claude/plugins`.
    ClaudePlugin,
}

impl SkillSource {
    /// Sources the overlay may rename/delete in place.
    pub fn editable(&self) -> bool {
        matches!(
            self,
            SkillSource::Jcode
                | SkillSource::Agents
                | SkillSource::ProjectJcode
                | SkillSource::ProjectAgents
        )
    }

    /// Sources shared with other agent tools on this machine.
    pub fn shared(&self) -> bool {
        matches!(self, SkillSource::Agents | SkillSource::ProjectAgents)
    }

    /// Source group header in the overlay.
    pub fn label(&self) -> String {
        match self {
            SkillSource::Jcode => "jcode · ~/.jcode/skills".into(),
            SkillSource::Agents => "~/.agents/skills (shared)".into(),
            SkillSource::ProjectJcode => "Project · .jcode/skills".into(),
            SkillSource::ProjectAgents => "Project · .agents/skills".into(),
            SkillSource::ProjectClaude => "Project · .claude/skills".into(),
            SkillSource::Plugin(name) => format!("Plugin · {name}"),
            SkillSource::ClaudePlugin => "Claude Code plugins".into(),
        }
    }
}

/// One skill directory.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillEntry {
    /// Frontmatter `name:` (falling back to the directory name).
    pub name: String,
    /// Frontmatter `description:` (may be empty).
    pub description: String,
    /// The skill directory itself.
    pub dir: PathBuf,
    pub source: SkillSource,
    /// `SKILL.md` present; `SKILL.md.disabled` alone means off.
    pub enabled: bool,
    pub editable: bool,
    /// Lives in a directory other tools also read (`~/.agents`, …).
    pub shared: bool,
}

/// List every skill across all sources.
pub fn list_skills(paths: &ExtensionPaths) -> Vec<SkillEntry> {
    let managed = plugin_skill_links(&paths.jcode_home);
    let mut skills = Vec::new();

    scan_skills_dir(
        &paths.jcode_home.join("skills"),
        SkillSource::Jcode,
        Some(&managed),
        &mut skills,
    );
    scan_skills_dir(
        &paths.user_home.join(".agents").join("skills"),
        SkillSource::Agents,
        None,
        &mut skills,
    );
    if let Some(project) = &paths.project_dir {
        for (relative, source) in [
            (".jcode/skills", SkillSource::ProjectJcode),
            (".agents/skills", SkillSource::ProjectAgents),
            (".claude/skills", SkillSource::ProjectClaude),
        ] {
            scan_skills_dir(&project.join(relative), source, None, &mut skills);
        }
    }
    for root in claude_plugin_skill_roots(&paths.user_home.join(".claude/plugins")) {
        scan_skills_dir(&root, SkillSource::ClaudePlugin, None, &mut skills);
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

/// `managed.skills` from the plugins state file: `plugin-<plugin>-<skill>`
/// link names mapped to their owning plugin.
fn plugin_skill_links(jcode_home: &Path) -> Vec<(String, String)> {
    let state_path = jcode_home.join("plugins").join(".state.json");
    let Ok(text) = std::fs::read_to_string(&state_path) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let plugin_names: Vec<String> = state
        .get("enabled")
        .and_then(serde_json::Value::as_object)
        .map(|enabled| enabled.keys().cloned().collect())
        .unwrap_or_default();
    let mut links = Vec::new();
    for link in state
        .get("managed")
        .and_then(|managed| managed.get("skills"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        // The state stores paths; only the link's file name identifies it.
        let name = Path::new(link)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| link.to_string());
        // `plugin-<plugin>-<skill>`; longest plugin-name prefix wins.
        let tail = name.strip_prefix("plugin-").unwrap_or(&name);
        let plugin = plugin_names
            .iter()
            .filter(|plugin| tail.starts_with(&format!("{plugin}-")))
            .max_by_key(|plugin| plugin.len())
            .cloned()
            .unwrap_or_else(|| tail.split('-').next().unwrap_or_default().to_string());
        links.push((name, plugin));
    }
    links
}

/// Scan one `skills/` root: immediate child dirs holding `SKILL.md`
/// (enabled) or `SKILL.md.disabled` (disabled).
fn scan_skills_dir(
    root: &Path,
    source: SkillSource,
    managed: Option<&[(String, String)]>,
    skills: &mut Vec<SkillEntry>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let enabled = dir.join("SKILL.md").is_file();
        if !enabled && !dir.join("SKILL.md.disabled").is_file() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let plugin = managed.and_then(|managed| {
            managed
                .iter()
                .find(|(link, _)| link == &dir_name)
                .map(|(_, plugin)| plugin.clone())
        });
        let source = plugin
            .clone()
            .map(SkillSource::Plugin)
            .unwrap_or_else(|| source.clone());
        let (name, description) = skill_meta(&dir, &dir_name, enabled);
        skills.push(SkillEntry {
            name,
            description,
            dir,
            enabled,
            editable: source.editable() && plugin.is_none(),
            shared: source.shared(),
            source,
        });
    }
}

/// `name:`/`description:` from the `---` frontmatter block; single-line
/// values, quotes stripped; name falls back to the directory name.
fn skill_meta(dir: &Path, dir_name: &str, enabled: bool) -> (String, String) {
    let file = if enabled {
        dir.join("SKILL.md")
    } else {
        dir.join("SKILL.md.disabled")
    };
    let Ok(content) = std::fs::read_to_string(file) else {
        return (dir_name.to_string(), String::new());
    };
    let mut name = String::new();
    let mut description = String::new();
    if let Some(block) = content
        .strip_prefix("---")
        .and_then(|rest| rest.split("\n---").next())
    {
        for line in block.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            match key.trim() {
                "name" if name.is_empty() => name = value,
                "description" if description.is_empty() => description = value,
                _ => {}
            }
        }
    }
    (
        if name.is_empty() {
            dir_name.to_string()
        } else {
            name
        },
        description,
    )
}

/// `skills/` roots inside Claude Code plugin installs: manifest install
/// paths plus the legacy `repos/` checkouts (mirrors `installed_plugin_paths`
/// in the runtime, without the deep `cache/` fallback scan).
fn claude_plugin_skill_roots(plugins_root: &Path) -> Vec<PathBuf> {
    if !plugins_root.is_dir() {
        return Vec::new();
    }
    let mut roots = Vec::new();
    if let Ok(text) = std::fs::read_to_string(plugins_root.join("installed_plugins.json"))
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(plugins) = value.get("plugins").and_then(|v| v.as_object())
    {
        for installs in plugins.values() {
            let list: Vec<&serde_json::Value> = match installs {
                serde_json::Value::Array(list) => list.iter().collect(),
                other => vec![other],
            };
            for install in list {
                if let Some(path) = install.get("installPath").and_then(|v| v.as_str()) {
                    let skills = Path::new(path).join("skills");
                    if skills.is_dir() {
                        roots.push(skills);
                    }
                }
            }
        }
    }
    if let Ok(repos) = std::fs::read_dir(plugins_root.join("repos")) {
        for owner in repos.flatten() {
            if let Ok(repos) = std::fs::read_dir(owner.path()) {
                for repo in repos.flatten() {
                    let skills = repo.path().join("skills");
                    if skills.is_dir() {
                        roots.push(skills);
                    }
                }
            }
        }
    }
    roots
}

// ---------------------------------------------------------------- edits ----

/// Toggle a skill by renaming `SKILL.md` ⇄ `SKILL.md.disabled`.
pub fn set_skill_enabled(entry: &SkillEntry, enabled: bool) -> Result<(), String> {
    let (from, to) = if enabled {
        (entry.dir.join("SKILL.md.disabled"), entry.dir.join("SKILL.md"))
    } else {
        (entry.dir.join("SKILL.md"), entry.dir.join("SKILL.md.disabled"))
    };
    if !from.is_file() {
        return if enabled && entry.dir.join("SKILL.md").is_file() {
            Ok(())
        } else {
            Err(format!("{} not found", from.display()))
        };
    }
    std::fs::rename(&from, &to).map_err(|error| {
        format!("Could not rename {}: {error}", from.display())
    })
}

/// Names the new-skill form accepts: `[a-z0-9][a-z0-9-]{0,63}`.
pub fn valid_skill_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    name.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Create `<root>/<name>/SKILL.md` with frontmatter and a stub body.
pub fn create_skill(root: &Path, name: &str, description: &str) -> Result<PathBuf, String> {
    if !valid_skill_name(name) {
        return Err(format!(
            "Skill names match [a-z0-9][a-z0-9-]{{0,63}}; `{name}` does not."
        ));
    }
    let dir = root.join(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let description = description.trim();
    let text = format!(
        "---\nname: {name}\ndescription: {}\n---\n\n# {name}\n\nDescribe when to use this skill and the steps it should follow.\n",
        if description.is_empty() { name } else { description }
    );
    std::fs::write(dir.join("SKILL.md"), text).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Copy a skill directory into `<dest_root>/<src dir name>`; refuses
/// existing targets and never follows directory symlinks.
pub fn import_skill(src: &Path, dest_root: &Path) -> Result<PathBuf, String> {
    if !src.join("SKILL.md").is_file() {
        return Err(format!("{} has no SKILL.md", src.display()));
    }
    let name = src
        .file_name()
        .ok_or_else(|| format!("{} has no directory name", src.display()))?;
    let dest = dest_root.join(name);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    copy_dir(src, &dest).map_err(|error| {
        format!("Could not import {}: {error}", src.display())
    })?;
    Ok(dest)
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_symlink() {
            // Never follow dir symlinks (loop risk); copy file targets.
            if path.is_dir() {
                continue;
            }
            if let Ok(resolved) = std::fs::canonicalize(&path) {
                std::fs::copy(resolved, &target)?;
            }
        } else if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Delete a skill by moving its directory to
/// `~/.jcode/trash/skills/<dir>-<unix>` — recoverable, never a hard remove.
pub fn delete_skill(entry: &SkillEntry, jcode_home: &Path) -> Result<PathBuf, String> {
    let trash = jcode_home.join("trash").join("skills");
    std::fs::create_dir_all(&trash).map_err(|error| error.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let dir_name = entry
        .dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.name.clone());
    let mut target = trash.join(format!("{dir_name}-{stamp}"));
    for attempt in 1.. {
        if !target.exists() {
            break;
        }
        target = trash.join(format!("{dir_name}-{stamp}-{attempt}"));
    }
    std::fs::rename(&entry.dir, &target).map_err(|error| {
        format!("Could not move {} to trash: {error}", entry.dir.display())
    })?;
    Ok(target)
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
