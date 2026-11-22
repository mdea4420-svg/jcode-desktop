//! Hook dispatch: one jcode hook entry per event runs every enabled plugin.
//!
//! A plugin's `hooks/hooks.json` uses Claude Code's shape:
//!
//! ```json
//! {"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "..."}]}]}}
//! ```
//!
//! Event keys may be Claude Code events (translated, see [`claude_event`]) or
//! jcode events (`pre_tool`, `turn_end`, ...), which receive jcode's own
//! environment and stdin unchanged. Matchers are regular expressions over the
//! tool name; Claude Code matchers see Claude Code tool names (`Bash`, `Edit`).
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::manifest::Plugin;

/// The jcode hook events, in config order.
pub const EVENTS: [&str; 8] = [
    "pre_tool_transform",
    "pre_tool",
    "post_tool",
    "post_tool_feedback",
    "turn_start",
    "turn_end",
    "session_start",
    "session_end",
];

/// The jcode event a Claude Code event runs on, if it has one.
pub fn claude_event(event: &str) -> Option<&'static str> {
    Some(match event {
        "PreToolUse" => "pre_tool",
        "PostToolUse" => "post_tool_feedback",
        "UserPromptSubmit" => "turn_start",
        "Stop" => "turn_end",
        "SessionStart" => "session_start",
        "SessionEnd" => "session_end",
        _ => return None,
    })
}

/// Claude Code's name for a jcode tool, for Claude Code matchers and payloads.
pub fn claude_tool_name(tool: &str) -> String {
    match tool {
        "bash" | "shell" => "Bash".into(),
        "read" | "read_file" => "Read".into(),
        "write" | "write_file" => "Write".into(),
        "edit" | "apply_patch" => "Edit".into(),
        "multiedit" | "multi_edit" => "MultiEdit".into(),
        "glob" => "Glob".into(),
        "grep" | "agentgrep" => "Grep".into(),
        "ls" => "LS".into(),
        "webfetch" | "web_fetch" => "WebFetch".into(),
        "websearch" | "web_search" => "WebSearch".into(),
        "todo" | "todowrite" => "TodoWrite".into(),
        "task" | "subagent" => "Task".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Flavor {
    /// A jcode event: environment and stdin pass through.
    Native,
    /// A Claude Code event, translated to its JSON-on-stdin contract.
    Claude(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HookEntry {
    pub plugin: String,
    pub root: PathBuf,
    pub flavor: Flavor,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout: Option<Duration>,
}

impl HookEntry {
    fn matches(&self, tool: &str) -> bool {
        let Some(pattern) = self.matcher.as_deref().map(str::trim) else {
            return true;
        };
        if pattern.is_empty() || pattern == "*" {
            return true;
        }
        let names = match self.flavor {
            Flavor::Native => vec![tool.to_string()],
            Flavor::Claude(_) => vec![claude_tool_name(tool), tool.to_string()],
        };
        match regex::Regex::new(&format!("^(?:{pattern})$")) {
            Ok(regex) => names.iter().any(|name| regex.is_match(name)),
            Err(_) => names.iter().any(|name| name == pattern),
        }
    }
}

fn parse_group(
    plugin: &Plugin,
    flavor: &Flavor,
    group: &Value,
    out: &mut Vec<HookEntry>,
) {
    let entry = |command: &str, matcher: Option<String>, timeout: Option<Duration>| HookEntry {
        plugin: plugin.name.clone(),
        root: plugin.root.clone(),
        flavor: flavor.clone(),
        matcher,
        command: command.to_string(),
        timeout,
    };
    match group {
        Value::String(command) => out.push(entry(command, None, None)),
        Value::Object(map) => {
            let matcher = map.get("matcher").and_then(Value::as_str).map(str::to_string);
            let hooks = map.get("hooks").and_then(Value::as_array).cloned().unwrap_or_else(|| {
                // A bare hook object (`{"command": ...}`) is its own group.
                vec![group.clone()]
            });
            for hook in hooks {
                let command = match &hook {
                    Value::String(command) => Some(command.as_str()),
                    Value::Object(hook) => hook
                        .get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|kind| kind == "command")
                        .then(|| hook.get("command").and_then(Value::as_str))
                        .flatten(),
                    _ => None,
                };
                let timeout = hook
                    .get("timeout")
                    .and_then(Value::as_f64)
                    .filter(|seconds| *seconds > 0.0)
                    .map(Duration::from_secs_f64);
                if let Some(command) = command.filter(|command| !command.trim().is_empty()) {
                    out.push(entry(command, matcher.clone(), timeout));
                }
            }
        }
        _ => {}
    }
}

/// The plugin's hooks, each with the jcode event it runs on. Unsupported
/// Claude Code events (`Notification`, `PreCompact`, ...) are reported.
pub fn plugin_hooks(plugin: &Plugin) -> anyhow::Result<(Vec<(&'static str, HookEntry)>, Vec<String>)> {
    let Some(document) = plugin.hooks_document()? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let events = document
        .get("hooks")
        .filter(|hooks| hooks.is_object())
        .unwrap_or(&document);
    let Some(events) = events.as_object() else {
        anyhow::bail!("{}: hooks must be an object", plugin.name);
    };
    let mut hooks = Vec::new();
    let mut unsupported = Vec::new();
    for (event, groups) in events {
        let (jcode_event, flavor) = if let Some(native) = EVENTS.iter().find(|e| *e == event) {
            (*native, Flavor::Native)
        } else if let Some(mapped) = claude_event(event) {
            (mapped, Flavor::Claude(event.clone()))
        } else {
            if event != "description" {
                unsupported.push(format!("{}: unsupported hook event `{event}`", plugin.name));
            }
            continue;
        };
        let groups = match groups {
            Value::Array(groups) => groups.clone(),
            other => vec![other.clone()],
        };
        let mut entries = Vec::new();
        for group in &groups {
            parse_group(plugin, &flavor, group, &mut entries);
        }
        hooks.extend(entries.into_iter().map(|entry| (jcode_event, entry)));
    }
    Ok((hooks, unsupported))
}

/// Everything the dispatcher reads from jcode for one invocation.
#[derive(Clone, Debug, Default)]
pub struct Invocation {
    pub event: String,
    pub session_id: String,
    pub cwd: Option<PathBuf>,
    pub tool_name: String,
    /// The tool input JSON that jcode wrote to stdin (empty for observers).
    pub stdin: Vec<u8>,
    pub source: String,
}

impl Invocation {
    /// Read jcode's hook environment and stdin.
    pub fn from_env(event: &str) -> Self {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let mut stdin = Vec::new();
        if matches!(event, "pre_tool" | "pre_tool_transform" | "post_tool_feedback") {
            let _ = std::io::stdin().read_to_end(&mut stdin);
        }
        Self {
            event: event.to_string(),
            session_id: var("JCODE_HOOK_SESSION_ID"),
            cwd: std::env::var_os("JCODE_HOOK_CWD")
                .map(PathBuf::from)
                .filter(|dir| dir.is_dir()),
            tool_name: var("JCODE_HOOK_TOOL_NAME"),
            stdin,
            source: var("JCODE_HOOK_SOURCE"),
        }
    }

    fn tool_input(&self) -> Value {
        serde_json::from_slice(&self.stdin).unwrap_or_else(|_| json!({}))
    }

    /// The JSON a Claude Code hook expects on stdin.
    fn claude_payload(&self, event: &str, input: &[u8]) -> Vec<u8> {
        let cwd = self
            .cwd
            .as_ref()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut payload = json!({
            "session_id": self.session_id,
            "transcript_path": "",
            "cwd": cwd,
            "hook_event_name": event,
            "permission_mode": "default",
        });
        let object = payload.as_object_mut().expect("object literal");
        match event {
            "PreToolUse" | "PostToolUse" => {
                object.insert("tool_name".into(), claude_tool_name(&self.tool_name).into());
                let input = serde_json::from_slice(input).unwrap_or_else(|_| self.tool_input());
                object.insert("tool_input".into(), input);
                if event == "PostToolUse" {
                    object.insert("tool_response".into(), json!({}));
                }
            }
            "UserPromptSubmit" => {
                object.insert("prompt".into(), "".into());
            }
            "Stop" => {
                object.insert("stop_hook_active".into(), false.into());
            }
            "SessionStart" => {
                let source = match self.source.as_str() {
                    "resume" | "attach" => "resume",
                    _ => "startup",
                };
                object.insert("source".into(), source.into());
            }
            "SessionEnd" => {
                object.insert("reason".into(), "other".into());
            }
            _ => {}
        }
        serde_json::to_vec(&payload).unwrap_or_default()
    }
}

#[derive(Debug, Default)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// The shell plugin hook commands run in. Claude Code hooks are shell
/// commands; on Windows Git Bash is preferred (never WSL's `bash.exe`), then
/// `cmd`. `JCODE_PLUGINS_SHELL` overrides, as a program taking `-c`.
fn shell() -> (String, Vec<String>) {
    if let Some(shell) = std::env::var_os("JCODE_PLUGINS_SHELL").filter(|s| !s.is_empty()) {
        return (shell.to_string_lossy().into_owned(), vec!["-c".into()]);
    }
    if cfg!(windows) {
        if let Some(bash) = git_bash() {
            return (bash.to_string_lossy().into_owned(), vec!["-c".into()]);
        }
        return ("cmd".into(), vec!["/D".into(), "/C".into()]);
    }
    ("sh".into(), vec!["-c".into()])
}

fn git_bash() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.join("git.exe").is_file())
        .flat_map(|dir| {
            let root = dir.parent().map(Path::to_path_buf);
            [root.as_ref().map(|r| r.join("bin").join("bash.exe")), Some(dir.join("bash.exe"))]
        })
        .flatten()
        .find(|bash| bash.is_file())
        .or_else(|| {
            [r"C:\Program Files\Git\bin\bash.exe", r"D:\Git\bin\bash.exe"]
                .iter()
                .map(PathBuf::from)
                .find(|bash| bash.is_file())
        })
}

/// Run one hook command with a timeout, feeding `stdin`.
pub fn run(entry: &HookEntry, invocation: &Invocation, stdin: &[u8], default_timeout: Duration) -> Output {
    let (program, mut args) = shell();
    let bash = !program.eq_ignore_ascii_case("cmd");
    let root = if bash {
        entry.root.to_string_lossy().replace('\\', "/")
    } else {
        entry.root.to_string_lossy().into_owned()
    };
    let command = entry
        .command
        .replace("${CLAUDE_PLUGIN_ROOT}", &root)
        .replace("${JCODE_PLUGIN_ROOT}", &root);
    args.push(command);
    let mut process = Command::new(&program);
    process
        .args(&args)
        .env("CLAUDE_PLUGIN_ROOT", &entry.root)
        .env("JCODE_PLUGIN_ROOT", &entry.root)
        .env("JCODE_PLUGIN_NAME", &entry.plugin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &invocation.cwd {
        process.current_dir(cwd).env("CLAUDE_PROJECT_DIR", cwd);
    }
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Output {
                stderr: format!("{}: could not start hook: {error}", entry.plugin),
                ..Default::default()
            };
        }
    };
    let input = stdin.to_vec();
    let mut child_stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(pipe) = child_stdin.as_mut() {
            let _ = pipe.write_all(&input);
        }
    });
    let reader = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let mut bytes = Vec::new();
                let _ = pipe.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let stdout = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let stderr = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let deadline = Instant::now() + entry.timeout.unwrap_or(default_timeout).min(default_timeout);
    let mut output = Output::default();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                output.code = status.code();
                break;
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                output.timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
    let _ = writer.join();
    output.stdout = stdout.join().unwrap_or_default();
    output.stderr = stderr.join().unwrap_or_default();
    output
}

fn default_timeout(event: &str) -> Duration {
    match event {
        "pre_tool_transform" => Duration::from_millis(500),
        "pre_tool" => Duration::from_secs(5),
        "post_tool_feedback" => Duration::from_secs(10),
        _ => Duration::from_secs(60),
    }
}

/// What the dispatcher prints and exits with.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A Claude Code hook's block decision: exit 2 with stderr, or JSON output.
fn claude_block(output: &Output) -> Option<String> {
    if output.code == Some(2) {
        return Some(output.stderr.trim().to_string());
    }
    if output.code != Some(0) {
        return None;
    }
    let json: Value = serde_json::from_str(output.stdout.trim()).ok()?;
    let reason = |key: &str, value: &Value| value.get(key).and_then(Value::as_str).map(str::to_string);
    if json.get("decision").and_then(Value::as_str) == Some("block") {
        return Some(reason("reason", &json).unwrap_or_else(|| "blocked by plugin".into()));
    }
    if json.get("continue").and_then(Value::as_bool) == Some(false) {
        return Some(reason("stopReason", &json).unwrap_or_else(|| "stopped by plugin".into()));
    }
    let specific = json.get("hookSpecificOutput")?;
    (specific.get("permissionDecision").and_then(Value::as_str) == Some("deny")).then(|| {
        reason("permissionDecisionReason", specific).unwrap_or_else(|| "denied by plugin".into())
    })
}

/// Text a Claude Code PostToolUse hook wants the model to see.
fn claude_feedback(output: &Output) -> Option<String> {
    if output.code == Some(2) {
        return Some(output.stderr.trim().to_string()).filter(|text| !text.is_empty());
    }
    let json: Value = serde_json::from_str(output.stdout.trim()).ok()?;
    let mut parts = Vec::new();
    if json.get("decision").and_then(Value::as_str) == Some("block")
        && let Some(reason) = json.get("reason").and_then(Value::as_str)
    {
        parts.push(reason.to_string());
    }
    if let Some(context) = json
        .get("hookSpecificOutput")
        .and_then(|specific| specific.get("additionalContext"))
        .and_then(Value::as_str)
    {
        parts.push(context.to_string());
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Run every matching entry for one jcode event and combine their results
/// according to the event's contract (see jcode `docs/HOOKS.md`).
pub fn dispatch(invocation: &Invocation, entries: &[HookEntry]) -> Outcome {
    let timeout = default_timeout(&invocation.event);
    let matching = entries
        .iter()
        .filter(|entry| invocation.tool_name.is_empty() || entry.matches(&invocation.tool_name));
    let mut outcome = Outcome::default();
    match invocation.event.as_str() {
        "pre_tool" => {
            for entry in matching {
                let output = match &entry.flavor {
                    Flavor::Native => run(entry, invocation, &invocation.stdin, timeout),
                    Flavor::Claude(event) => {
                        let payload = invocation.claude_payload(event, &invocation.stdin);
                        run(entry, invocation, &payload, timeout)
                    }
                };
                let blocked = match &entry.flavor {
                    Flavor::Native => (output.code == Some(2)).then(|| output.stderr.trim().to_string()),
                    Flavor::Claude(_) => claude_block(&output),
                };
                if let Some(reason) = blocked {
                    let reason = if reason.is_empty() {
                        format!("blocked by plugin {}", entry.plugin)
                    } else {
                        reason
                    };
                    return Outcome {
                        code: 2,
                        stdout: String::new(),
                        stderr: format!("[{}] {reason}", entry.plugin),
                    };
                }
            }
        }
        "pre_tool_transform" => {
            let mut current = invocation.stdin.clone();
            let mut changed = false;
            for entry in matching.filter(|entry| entry.flavor == Flavor::Native) {
                let output = run(entry, invocation, &current, timeout);
                let replacement = output.stdout.trim();
                if output.code == Some(0)
                    && serde_json::from_str::<Value>(replacement).is_ok_and(|v| v.is_object())
                {
                    current = replacement.as_bytes().to_vec();
                    changed = true;
                }
            }
            if changed {
                outcome.stdout = String::from_utf8_lossy(&current).into_owned();
            }
        }
        "post_tool_feedback" => {
            let mut parts = Vec::new();
            for entry in matching {
                let feedback = match &entry.flavor {
                    Flavor::Native => {
                        let output = run(entry, invocation, &invocation.stdin, timeout);
                        Some(output.stdout.trim().to_string()).filter(|text| !text.is_empty())
                    }
                    Flavor::Claude(event) => {
                        let payload = invocation.claude_payload(event, &invocation.stdin);
                        claude_feedback(&run(entry, invocation, &payload, timeout))
                    }
                };
                parts.extend(feedback.map(|text| format!("[{}] {text}", entry.plugin)));
            }
            outcome.stdout = parts.join("\n");
        }
        _ => {
            // Observers: jcode does not wait on these. Run every entry.
            for entry in matching {
                let stdin = match &entry.flavor {
                    Flavor::Native => invocation.stdin.clone(),
                    Flavor::Claude(event) => invocation.claude_payload(event, &[]),
                };
                let output = run(entry, invocation, &stdin, timeout);
                if output.timed_out || output.code.is_some_and(|code| code != 0) {
                    outcome
                        .stderr
                        .push_str(&format!("[{}] hook failed: {}\n", entry.plugin, output.stderr.trim()));
                }
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Source;

    fn plugin_with_hooks(dir: &Path, hooks: &str) -> Plugin {
        std::fs::create_dir_all(dir.join("hooks")).unwrap();
        std::fs::write(dir.join("hooks/hooks.json"), hooks).unwrap();
        let mut plugin = Plugin::load(dir, Source::Jcode).unwrap();
        plugin.enabled = true;
        plugin
    }

    #[test]
    fn parses_claude_and_native_events() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = plugin_with_hooks(
            &temp.path().join("guard"),
            r#"{"description": "x", "hooks": {
                "PreToolUse": [{"matcher": "Bash|Write", "hooks": [{"type": "command", "command": "guard.sh", "timeout": 3}]}],
                "turn_end": ["notify"],
                "Notification": [{"hooks": [{"type": "command", "command": "n"}]}]
            }}"#,
        );
        let (hooks, unsupported) = plugin_hooks(&plugin).unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].0, "pre_tool");
        assert_eq!(hooks[0].1.flavor, Flavor::Claude("PreToolUse".into()));
        assert_eq!(hooks[0].1.timeout, Some(Duration::from_secs(3)));
        assert!(hooks[0].1.matches("bash"));
        assert!(hooks[0].1.matches("write"));
        assert!(!hooks[0].1.matches("read"));
        assert_eq!(hooks[1].0, "turn_end");
        assert_eq!(hooks[1].1.flavor, Flavor::Native);
        assert_eq!(unsupported.len(), 1);
    }

    #[test]
    fn claude_decisions_block_and_feed_back() {
        let exit2 = Output { code: Some(2), stderr: "no rm".into(), ..Default::default() };
        assert_eq!(claude_block(&exit2).as_deref(), Some("no rm"));
        let deny = Output {
            code: Some(0),
            stdout: r#"{"hookSpecificOutput": {"permissionDecision": "deny", "permissionDecisionReason": "nope"}}"#.into(),
            ..Default::default()
        };
        assert_eq!(claude_block(&deny).as_deref(), Some("nope"));
        let allow = Output { code: Some(0), stdout: "ok".into(), ..Default::default() };
        assert_eq!(claude_block(&allow), None);
        let failed = Output { code: Some(1), stderr: "crash".into(), ..Default::default() };
        assert_eq!(claude_block(&failed), None, "other exit codes fail open");
        let context = Output {
            code: Some(0),
            stdout: r#"{"hookSpecificOutput": {"additionalContext": "lint: 2 issues"}}"#.into(),
            ..Default::default()
        };
        assert_eq!(claude_feedback(&context).as_deref(), Some("lint: 2 issues"));
    }

    #[test]
    fn claude_payload_uses_claude_tool_names() {
        let invocation = Invocation {
            event: "pre_tool".into(),
            session_id: "s1".into(),
            tool_name: "bash".into(),
            stdin: br#"{"command": "ls"}"#.to_vec(),
            ..Default::default()
        };
        let payload: Value =
            serde_json::from_slice(&invocation.claude_payload("PreToolUse", &invocation.stdin)).unwrap();
        assert_eq!(payload["tool_name"], "Bash");
        assert_eq!(payload["tool_input"]["command"], "ls");
        assert_eq!(payload["hook_event_name"], "PreToolUse");
    }

    #[cfg(unix)]
    #[test]
    fn dispatch_blocks_transforms_and_collects_feedback() {
        let temp = tempfile::tempdir().unwrap();
        let entry = |command: &str, flavor: Flavor| HookEntry {
            plugin: "p".into(),
            root: temp.path().to_path_buf(),
            flavor,
            matcher: Some("bash".into()),
            command: command.into(),
            timeout: None,
        };
        let invocation = |event: &str| Invocation {
            event: event.into(),
            tool_name: "bash".into(),
            stdin: br#"{"command":"rm -rf /"}"#.to_vec(),
            ..Default::default()
        };
        let block = entry("grep -q 'rm -rf' && echo 'destructive' >&2 && exit 2; exit 0", Flavor::Native);
        let outcome = dispatch(&invocation("pre_tool"), &[block]);
        assert_eq!(outcome.code, 2);
        assert_eq!(outcome.stderr, "[p] destructive");

        let claude = entry(
            r#"cat >/dev/null; echo '{"decision":"block","reason":"policy"}'"#,
            Flavor::Claude("PreToolUse".into()),
        );
        assert_eq!(dispatch(&invocation("pre_tool"), &[claude]).stderr, "[p] policy");

        let rewrite = entry(r#"cat >/dev/null; echo '{"command":"ls"}'"#, Flavor::Native);
        let outcome = dispatch(&invocation("pre_tool_transform"), &[rewrite]);
        assert_eq!(outcome.stdout, r#"{"command":"ls"}"#);

        let lint = entry("cat >/dev/null; echo 'looks fine'", Flavor::Native);
        assert_eq!(dispatch(&invocation("post_tool_feedback"), &[lint]).stdout, "[p] looks fine");

        let other_tool = Invocation { tool_name: "read".into(), ..invocation("pre_tool") };
        let block = entry("exit 2", Flavor::Native);
        assert_eq!(dispatch(&other_tool, &[block]).code, 0, "matcher filters tools");
    }
}
