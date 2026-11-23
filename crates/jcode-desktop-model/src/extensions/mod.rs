//! Extensions manager data: jcode MCP servers and skills.
//!
//! GPUI-free file I/O for the Desktop "Extensions" overlay. Everything takes
//! an [`ExtensionPaths`] so tests and the offline screenshot fixture run
//! against temp directories and never touch the real `~/.jcode`.

use std::path::PathBuf;

pub mod mcp;
pub mod skills;

/// The three roots extensions are read from and written to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionPaths {
    /// `~/.jcode` (or `$JCODE_HOME`).
    pub jcode_home: PathBuf,
    /// The user's home directory (for `~/.claude.json`, `~/.agents/skills`).
    pub user_home: PathBuf,
    /// The session working directory; project-local sources only exist when
    /// this is set.
    pub project_dir: Option<PathBuf>,
}

impl ExtensionPaths {
    /// Production paths: jcode home, user home, and the panel's working dir.
    pub fn detect(project_dir: Option<PathBuf>) -> Option<Self> {
        Some(Self {
            jcode_home: crate::platform_dirs::jcode_home()?,
            user_home: crate::platform_dirs::home_dir()?,
            project_dir,
        })
    }
}

/// Which tab the Extensions overlay shows. `Last` resolves to the tab the
/// overlay was closed on, so `/extensions` and the sidebar button reopen
/// where the user left off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExtensionTab {
    Mcp,
    #[default]
    Skills,
    Plugins,
    /// Reopen whichever tab was active last.
    Last,
}
