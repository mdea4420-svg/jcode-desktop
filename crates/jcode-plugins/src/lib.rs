//! Plugin packages for Jcode.
//!
//! A plugin is a directory under `~/.jcode/plugins/<name>/`, laid out like a
//! Claude Code plugin so existing ones work unchanged:
//!
//! ```text
//! my-plugin/
//!   .claude-plugin/plugin.json   name, version, description (optional)
//!   commands/*.md                slash commands (prompt templates)
//!   skills/<skill>/SKILL.md      skills
//!   hooks/hooks.json             lifecycle hooks (Claude Code or jcode events)
//!   .mcp.json                    MCP servers
//!   applets/<id>/applet.json     Jcode Desktop applets (UI)
//! ```
//!
//! Installed Claude Code plugins (`~/.claude/plugins/installed_plugins.json`)
//! are discovered too, disabled until enabled.
//!
//! The jcode runtime already loads skills, hooks and MCP servers from its own
//! config, so [`sync`] links and registers the enabled plugins there. Hooks go
//! through one dispatcher entry per event (`jcode-plugins hook <event>`),
//! which fans out to every enabled plugin and translates Claude Code hooks.

pub mod commands;
pub mod hooks;
pub mod manifest;
pub mod paths;
pub mod scaffold;
pub mod state;
pub mod sync;
mod toml_edit;

pub use commands::{Catalog, PluginCommand};
pub use manifest::{Plugin, Source, discover};
pub use paths::Layout;
pub use state::State;
pub use sync::{SyncOptions, SyncReport, sync};
