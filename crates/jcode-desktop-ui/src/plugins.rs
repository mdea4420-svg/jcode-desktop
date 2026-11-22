//! Jcode plugins in Desktop: slash commands, applets, and config sync.
//!
//! Discovery, command templates, hook dispatch and syncing live in the
//! GPUI-free `jcode-plugins` crate (also the `jcode-plugins` CLI shipped next
//! to this executable). Desktop keeps a catalog of enabled plugin commands for
//! the composer and syncs the runtime config once at startup.
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use jcode_plugins::{Catalog, Layout, Plugin, State, SyncOptions, SyncReport};

static CATALOG: RwLock<Option<Arc<Catalog>>> = RwLock::new(None);

/// Desktop's own slash commands that a plugin must not take over.
fn reserved(name: &str) -> bool {
    crate::commands::registered_command(name).is_some()
        || matches!(
            name,
            "/plugins" | "/plugin" | "/thinking" | "/reasoning" | "/diff" | "/version" | "/record"
        )
}

pub(crate) fn layout() -> Layout {
    Layout::detect()
}

/// Every discovered plugin with its enabled flag.
pub(crate) fn plugins() -> (Vec<Plugin>, Vec<String>) {
    let layout = layout();
    jcode_plugins::discover(&layout, &State::load(&layout))
}

/// Rebuild the command catalog from disk.
pub(crate) fn reload() -> Arc<Catalog> {
    let (plugins, _) = plugins();
    let catalog = Arc::new(Catalog::build(&plugins, reserved));
    *CATALOG.write().unwrap_or_else(|error| error.into_inner()) = Some(catalog.clone());
    catalog
}

pub(crate) fn catalog() -> Arc<Catalog> {
    if cfg!(test) {
        return Arc::default();
    }
    if let Some(catalog) = CATALOG
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        return catalog;
    }
    reload()
}

/// `(slash name, help)` rows for the composer's command menu.
pub(crate) fn command_entries() -> Vec<(String, String)> {
    catalog().entries()
}

/// The prompt a composer line expands to, if it is a plugin command.
pub(crate) fn expand(input: &str) -> Option<String> {
    catalog().expand(input)
}

/// The `jcode-plugins` CLI next to this executable: the hook dispatcher.
pub(crate) fn dispatcher() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "jcode-plugins.exe"
    } else {
        "jcode-plugins"
    };
    let path = exe.parent()?.join(name);
    path.is_file().then_some(path)
}

pub(crate) fn sync_now() -> SyncReport {
    let report = jcode_plugins::sync(
        &layout(),
        &SyncOptions {
            dispatcher: dispatcher(),
        },
    );
    reload();
    report
}

/// Sync the runtime config once per process, off the UI thread.
pub(crate) fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if cfg!(test) || crate::harness::screenshot_mode() || STARTED.set(()).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("plugin-sync".into())
        .spawn(|| {
            let report = sync_now();
            for warning in &report.warnings {
                eprintln!("jcode desktop: plugins: {warning}");
            }
        });
}

/// `applets/` folders of enabled plugins, started like `~/.jcode/applets`.
pub(crate) fn applet_dirs() -> Vec<PathBuf> {
    let (plugins, _) = plugins();
    plugins
        .iter()
        .filter(|plugin| plugin.enabled)
        .flat_map(Plugin::applet_dirs)
        .collect()
}

/// Markdown for `/plugins`.
pub(crate) fn summary_markdown() -> String {
    let layout = layout();
    let (plugins, errors) = plugins();
    let catalog = catalog();
    let mut text = String::from("**Plugins**\n\n");
    if plugins.is_empty() {
        text.push_str(&format!(
            "No plugins yet. Put them in `{}`, or run `/plugins new <name>` for a starter.\n",
            layout.plugins_dir.display()
        ));
    }
    for plugin in &plugins {
        let commands: Vec<String> = catalog
            .commands()
            .iter()
            .filter(|command| command.plugin == plugin.name)
            .map(|command| format!("`/{}`", command.qualified()))
            .collect();
        text.push_str(&format!(
            "- **{}** {} · {} · {}{}\n",
            plugin.name,
            plugin.version.as_deref().unwrap_or(""),
            if plugin.enabled { "on" } else { "off" },
            plugin.source.label(),
            plugin
                .description
                .as_deref()
                .map(|description| format!(" · {description}"))
                .unwrap_or_default()
        ));
        if !commands.is_empty() {
            text.push_str(&format!("  - commands: {}\n", commands.join(", ")));
        }
        let skills: Vec<String> = plugin.skills().into_iter().map(|(name, _)| name).collect();
        if !skills.is_empty() {
            text.push_str(&format!("  - skills: {}\n", skills.join(", ")));
        }
    }
    for error in errors {
        text.push_str(&format!("- error: {error}\n"));
    }
    text.push_str(
        "\n`/plugins enable <name>` · `/plugins disable <name>` · `/plugins new <name>` · `/plugins reload`",
    );
    if dispatcher().is_none() {
        text.push_str("\n\nHooks need `jcode-plugins` next to Jcode Desktop; commands, skills, MCP servers and applets work without it.");
    }
    text
}

/// Enable or disable a plugin and apply it.
pub(crate) fn set_enabled(name: &str, enabled: bool) -> Result<SyncReport, String> {
    let layout = layout();
    let mut state = State::load(&layout);
    let (plugins, _) = jcode_plugins::discover(&layout, &state);
    if !plugins.iter().any(|plugin| plugin.name == name) {
        return Err(format!("No plugin named `{name}`. Run `/plugins` to list them."));
    }
    state.set_enabled(name, enabled);
    state.save(&layout).map_err(|error| error.to_string())?;
    Ok(sync_now())
}

pub(crate) fn create(name: &str) -> Result<PathBuf, String> {
    let root = jcode_plugins::scaffold::create(&layout().plugins_dir, name)
        .map_err(|error| format!("{error:#}"))?;
    sync_now();
    Ok(root)
}

pub(crate) fn report_markdown(report: &SyncReport) -> String {
    let mut text = format!(
        "Synced {} enabled of {} plugins: {} skills, {} MCP servers, hooks on {}.",
        report.enabled,
        report.plugins,
        report.skills,
        report.mcp_servers,
        if report.hook_events.is_empty() {
            "no events".to_string()
        } else {
            report.hook_events.join(", ")
        }
    );
    for warning in &report.warnings {
        text.push_str(&format!("\n- warning: {warning}"));
    }
    if report.changed {
        text.push_str("\n\nNew skills, MCP servers and hooks apply to new sessions (or after the runtime reloads its config).");
    }
    text
}
