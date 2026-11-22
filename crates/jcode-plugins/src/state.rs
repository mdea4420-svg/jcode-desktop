use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::manifest::Source;
use crate::paths::Layout;

/// Enabled flags and what [`crate::sync`] wrote into the runtime config, so a
/// later sync removes exactly its own entries and never the user's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
    #[serde(default)]
    pub managed: Managed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Managed {
    /// Links created in `~/.jcode/skills`.
    #[serde(default)]
    pub skills: Vec<PathBuf>,
    /// Server names added to `~/.jcode/mcp.json`.
    #[serde(default)]
    pub mcp: Vec<String>,
}

impl State {
    pub fn load(layout: &Layout) -> Self {
        std::fs::read_to_string(layout.state_file())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, layout: &Layout) -> anyhow::Result<()> {
        std::fs::create_dir_all(&layout.plugins_dir)?;
        crate::sync::write_atomic(
            &layout.state_file(),
            serde_json::to_string_pretty(self)?.as_bytes(),
        )
    }

    /// Jcode plugins are on unless disabled. Claude Code plugins are off until
    /// enabled, since their hooks were written for another agent.
    pub fn is_enabled(&self, name: &str, source: Source) -> bool {
        self.enabled
            .get(name)
            .copied()
            .unwrap_or(source == Source::Jcode)
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        self.enabled.insert(name.to_string(), enabled);
    }
}
