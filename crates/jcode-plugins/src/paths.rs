use std::path::{Path, PathBuf};

/// Where plugins and the jcode runtime config live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// `~/.jcode` (or `$JCODE_HOME`).
    pub jcode_home: PathBuf,
    /// `~/.jcode/plugins` (or `$JCODE_PLUGINS_DIR`).
    pub plugins_dir: PathBuf,
    /// `~/.claude/plugins`, when Claude Code plugins should be discovered.
    pub claude_plugins: Option<PathBuf>,
}

pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .or_else(|| std::env::var_os("HOME"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

impl Layout {
    /// The user's real layout, honoring `JCODE_HOME` and `JCODE_PLUGINS_DIR`.
    pub fn detect() -> Self {
        let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
        let jcode_home = std::env::var_os("JCODE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".jcode"));
        let plugins_dir = std::env::var_os("JCODE_PLUGINS_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| jcode_home.join("plugins"));
        Self {
            jcode_home,
            plugins_dir,
            claude_plugins: Some(home.join(".claude").join("plugins")),
        }
    }

    /// A self-contained layout rooted at `root`, for tests.
    pub fn at(root: &Path) -> Self {
        Self {
            jcode_home: root.join(".jcode"),
            plugins_dir: root.join(".jcode").join("plugins"),
            claude_plugins: Some(root.join(".claude").join("plugins")),
        }
    }

    pub fn config_toml(&self) -> PathBuf {
        self.jcode_home.join("config.toml")
    }

    pub fn mcp_json(&self) -> PathBuf {
        self.jcode_home.join("mcp.json")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.jcode_home.join("skills")
    }

    pub fn state_file(&self) -> PathBuf {
        self.plugins_dir.join(".state.json")
    }
}

/// A path as a command-line program token for jcode hook commands. Hooks are
/// parsed shell-style, so Windows backslashes would read as escapes: use
/// forward slashes, which Windows accepts, and quote paths with spaces.
pub fn command_token(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_tokens_survive_shell_style_parsing() {
        let plain = command_token(Path::new(r"F:\jcode\Jcode\jcode-plugins.exe"));
        assert_eq!(plain, "F:/jcode/Jcode/jcode-plugins.exe");
        let spaced = command_token(Path::new(r"C:\Program Files\Jcode\jcode-plugins.exe"));
        let parsed = shlex::split(&format!("{spaced} hook pre_tool")).unwrap();
        assert_eq!(parsed[0], "C:/Program Files/Jcode/jcode-plugins.exe");
        assert_eq!(&parsed[1..], ["hook", "pre_tool"]);
    }
}
