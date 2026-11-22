//! `jcode-plugins`: manage Jcode plugins, and dispatch their hooks.
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jcode_plugins::{Catalog, Layout, State, SyncOptions, discover, hooks, scaffold};

const USAGE: &str = "\
jcode-plugins: Jcode plugin manager

USAGE:
  jcode-plugins list                      Installed plugins and their state
  jcode-plugins info <name>               Commands, skills, hooks and MCP servers
  jcode-plugins enable <name>             Enable a plugin, then sync
  jcode-plugins disable <name>            Disable a plugin, then sync
  jcode-plugins install <path|git-url>    Copy or clone a plugin into ~/.jcode/plugins
  jcode-plugins uninstall <name>          Delete a plugin from ~/.jcode/plugins
  jcode-plugins new <name> [--code]       Create a starter plugin (--code: JavaScript plugin)
  jcode-plugins sync                      Apply enabled plugins to the jcode config
  jcode-plugins run </command args>       Print the prompt a plugin command expands to
  jcode-plugins hook <event>              (internal) hook dispatcher used by jcode
";

fn options() -> SyncOptions {
    SyncOptions {
        dispatcher: std::env::current_exe().ok(),
    }
}

fn find<'a>(plugins: &'a [jcode_plugins::Plugin], name: &str) -> anyhow::Result<&'a jcode_plugins::Plugin> {
    plugins
        .iter()
        .find(|plugin| plugin.name == name)
        .ok_or_else(|| anyhow::anyhow!("no plugin named `{name}` (see `jcode-plugins list`)"))
}

fn print_sync(report: &jcode_plugins::SyncReport) {
    println!(
        "Synced {} enabled of {} plugins: {} skills, {} MCP servers, hooks on [{}]",
        report.enabled,
        report.plugins,
        report.skills,
        report.mcp_servers,
        report.hook_events.join(", ")
    );
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
}

fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn is_git_url(source: &str) -> bool {
    source.starts_with("https://")
        || source.starts_with("http://")
        || source.starts_with("git@")
        || source.ends_with(".git")
}

fn install(layout: &Layout, source: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(&layout.plugins_dir)?;
    let staging = layout.plugins_dir.join(format!(".install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    if is_git_url(source) {
        let status = std::process::Command::new("git")
            .args(["clone", "--depth", "1", source])
            .arg(&staging)
            .status()?;
        anyhow::ensure!(status.success(), "git clone failed");
        let _ = std::fs::remove_dir_all(staging.join(".git"));
    } else {
        let path = Path::new(source);
        anyhow::ensure!(path.is_dir(), "{source} is not a directory or git URL");
        copy_dir(path, &staging)?;
    }
    let plugin = match jcode_plugins::Plugin::load(&staging, jcode_plugins::Source::Jcode) {
        Ok(plugin) => plugin,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let target = layout.plugins_dir.join(&plugin.name);
    if target.exists() {
        let _ = std::fs::remove_dir_all(&staging);
        anyhow::bail!(
            "{} is already installed; run `jcode-plugins uninstall {}` first",
            plugin.name,
            plugin.name
        );
    }
    std::fs::rename(&staging, &target)?;
    Ok(target)
}

fn run(args: &[String]) -> anyhow::Result<ExitCode> {
    let layout = Layout::detect();
    let command = args.first().map(String::as_str).unwrap_or("help");
    let arg = args.get(1).map(String::as_str);
    match (command, arg) {
        ("hook", Some(event)) => {
            // Hooks must never break the agent: report problems on stderr and
            // fall back to "allow" (exit 0) like jcode's own fail-open hooks.
            let state = State::load(&layout);
            let (plugins, _) = discover(&layout, &state);
            let mut entries: Vec<_> = plugins
                .iter()
                .filter(|plugin| plugin.enabled)
                .filter_map(|plugin| hooks::plugin_hooks(plugin).ok())
                .flat_map(|(entries, _)| entries)
                .filter(|(hook_event, _)| *hook_event == event)
                .map(|(_, entry)| entry)
                .collect();
            entries.extend(jcode_plugins::code::hook_entries(&layout, &plugins, event));
            if entries.is_empty() {
                return Ok(ExitCode::SUCCESS);
            }
            let invocation = hooks::Invocation::from_env(event);
            let outcome = hooks::dispatch(&invocation, &entries);
            print!("{}", outcome.stdout);
            eprint!("{}", outcome.stderr);
            let _ = std::io::stdout().flush();
            return Ok(ExitCode::from(outcome.code as u8));
        }
        ("list", _) => {
            let state = State::load(&layout);
            let (plugins, errors) = discover(&layout, &state);
            if plugins.is_empty() {
                println!("No plugins in {}. Create one with `jcode-plugins new <name>`.", layout.plugins_dir.display());
            }
            for plugin in &plugins {
                println!(
                    "{} {:<24} {:<8} {:<7} {}",
                    if plugin.enabled { "[on] " } else { "[off]" },
                    plugin.name,
                    plugin.version.as_deref().unwrap_or("-"),
                    plugin.source.label(),
                    plugin.description.as_deref().unwrap_or("")
                );
            }
            for error in errors {
                eprintln!("error: {error}");
            }
        }
        ("info", Some(name)) => {
            let state = State::load(&layout);
            let (plugins, _) = discover(&layout, &state);
            let plugin = find(&plugins, name)?;
            println!("{} {} ({})", plugin.name, plugin.version.as_deref().unwrap_or(""), plugin.source.label());
            println!("  path:    {}", plugin.root.display());
            println!("  enabled: {}", plugin.enabled);
            let mut single = plugin.clone();
            single.enabled = true;
            for command in Catalog::build([&single], |_| false).commands() {
                println!("  command: /{}  {}", command.qualified(), command.description.as_deref().unwrap_or(""));
            }
            for (skill, _) in plugin.skills() {
                println!("  skill:   {skill}");
            }
            let (entries, unsupported) = hooks::plugin_hooks(plugin)?;
            for (event, entry) in entries {
                println!("  hook:    {event} ({:?}) {}", entry.flavor, entry.command);
            }
            for warning in unsupported {
                println!("  warning: {warning}");
            }
            for server in plugin.mcp_servers()?.keys() {
                println!("  mcp:     {server}");
            }
        }
        ("enable" | "disable", Some(name)) => {
            let mut state = State::load(&layout);
            let (plugins, _) = discover(&layout, &state);
            find(&plugins, name)?;
            state.set_enabled(name, command == "enable");
            state.save(&layout)?;
            print_sync(&jcode_plugins::sync(&layout, &options()));
        }
        ("install", Some(source)) => {
            let target = install(&layout, source)?;
            println!("Installed {}", target.display());
            print_sync(&jcode_plugins::sync(&layout, &options()));
        }
        ("uninstall", Some(name)) => {
            let state = State::load(&layout);
            let (plugins, _) = discover(&layout, &state);
            let plugin = find(&plugins, name)?;
            anyhow::ensure!(
                plugin.source == jcode_plugins::Source::Jcode,
                "{name} is a Claude Code plugin; disable it here or uninstall it in Claude Code"
            );
            std::fs::remove_dir_all(&plugin.root)?;
            println!("Removed {}", plugin.root.display());
            print_sync(&jcode_plugins::sync(&layout, &options()));
        }
        ("new", Some(name)) => {
            let root = if args.iter().any(|arg| arg == "--code") {
                scaffold::create_code(&layout.plugins_dir, name)?
            } else {
                scaffold::create(&layout.plugins_dir, name)?
            };
            println!("Created {}", root.display());
            print_sync(&jcode_plugins::sync(&layout, &options()));
        }
        ("sync", _) => print_sync(&jcode_plugins::sync(&layout, &options())),
        ("run", Some(_)) => {
            let state = State::load(&layout);
            let (plugins, _) = discover(&layout, &state);
            let catalog = Catalog::build(&plugins, |_| false);
            let line = args[1..].join(" ");
            let Some((command, rest)) = catalog.find(&line) else {
                anyhow::bail!("no enabled plugin command matches `{line}`");
            };
            if command.code {
                let plugin = find(&plugins, &command.plugin)?;
                let cwd = std::env::current_dir().ok();
                let reply = jcode_plugins::code::run_command(&layout, plugin, &command.name, rest, cwd.as_deref())?;
                if let Some(error) = reply.error {
                    anyhow::bail!("{error}");
                }
                println!("{}", reply.prompt.or(reply.message).unwrap_or_default());
            } else {
                println!("{}", command.expand(rest));
            }
        }
        _ => {
            print!("{USAGE}");
            return Ok(if matches!(command, "help" | "--help" | "-h") {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            });
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hook = args.first().is_some_and(|command| command == "hook");
    match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("jcode-plugins: {error:#}");
            if hook { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
    }
}
