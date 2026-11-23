use super::*;
use std::fs;

fn paths(root: &Path) -> ExtensionPaths {
    ExtensionPaths {
        jcode_home: root.join(".jcode"),
        user_home: root.join("home"),
        project_dir: Some(root.join("project")),
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn lists_every_source_with_plugin_owned_marked() {
    let root = tempfile::tempdir().unwrap();
    let p = paths(root.path());
    write(
        &p.jcode_home.join("mcp.json"),
        r#"{"mcpServers":{"local":{"command":"npx","args":["srv"]},"tools-main":{"command":"run"}}}"#,
    );
    write(
        &p.jcode_home.join("plugins/.state.json"),
        r#"{"enabled":{"tools":true},"managed":{"mcp":["tools-main"],"skills":[]}}"#,
    );
    write(
        &p.user_home.join(".claude.json"),
        &format!(
            r#"{{"mcpServers":{{"claude-global":{{"url":"https://x"}}}},"projects":{{"{}":{{"mcpServers":{{"claude-proj":{{"command":"cp"}}}}}}}}}}"#,
            p.project_dir.as_ref().unwrap().to_string_lossy().replace('\\', "\\\\")
        ),
    );
    write(
        &p.user_home.join(".claude/mcp.json"),
        r#"{"mcpServers":{"legacy":{"command":"old"}}}"#,
    );
    let project = p.project_dir.as_ref().unwrap();
    write(
        &project.join(".jcode/mcp.json"),
        r#"{"mcpServers":{"proj":{"command":"pj","disabled":true}}}"#,
    );
    write(
        &project.join(".mcp.json"),
        r#"{"mcpServers":{"dotmcp":{"url":"https://m"}}}"#,
    );
    write(
        &project.join(".claude/mcp.json"),
        r#"{"mcpServers":{"pc":{"command":"c","enabled":false}}}"#,
    );

    let list = list_mcp(&p);
    assert!(list.notices.is_empty(), "{:?}", list.notices);
    let by_name: std::collections::HashMap<_, _> = list
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    assert_eq!(by_name.len(), 8, "{:?}", by_name.keys().collect::<Vec<_>>());
    assert_eq!(by_name["local"].source, McpSource::JcodeGlobal);
    assert!(by_name["local"].editable);
    assert_eq!(by_name["tools-main"].plugin.as_deref(), Some("tools"));
    assert!(!by_name["tools-main"].editable);
    assert_eq!(by_name["claude-global"].source, McpSource::ClaudeUser);
    assert_eq!(by_name["claude-global"].transport, McpTransport::Http);
    assert!(!by_name["claude-global"].editable);
    assert_eq!(by_name["claude-proj"].source, McpSource::ClaudeProject);
    assert_eq!(by_name["legacy"].source, McpSource::ClaudeUser);
    assert!(!by_name["proj"].enabled);
    assert_eq!(by_name["dotmcp"].source, McpSource::ProjectMcp);
    assert_eq!(by_name["dotmcp"].transport, McpTransport::Http);
    assert!(!by_name["pc"].enabled);
}

#[test]
fn invalid_files_become_notices_not_panics() {
    let root = tempfile::tempdir().unwrap();
    let p = paths(root.path());
    write(&p.jcode_home.join("mcp.json"), "{oops");
    write(&p.user_home.join(".claude.json"), "not json");
    let list = list_mcp(&p);
    assert_eq!(list.notices.len(), 2);
    assert!(list.entries.is_empty());
}

#[test]
fn enable_disable_semantics() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.json");
    write(
        &path,
        r#"{"mcpServers":{"a":{"command":"x","enabled":false},"b":{"command":"y","enabled":true,"disabled":true}}}"#,
    );
    // Disabling writes `disabled` and removes `enabled`.
    set_mcp_enabled(&path, "a", false).unwrap();
    let doc = read_doc(&path).unwrap();
    let a = &doc["mcpServers"]["a"];
    assert_eq!(a["disabled"], true);
    assert!(a.get("enabled").is_none());
    // Enabling removes `disabled` and any `enabled: false`, keeps `enabled: true`.
    set_mcp_enabled(&path, "b", true).unwrap();
    let doc = read_doc(&path).unwrap();
    let b = &doc["mcpServers"]["b"];
    assert!(b.get("disabled").is_none());
    assert_eq!(b["enabled"], true);
    set_mcp_enabled(&path, "a", true).unwrap();
    let doc = read_doc(&path).unwrap();
    assert!(doc["mcpServers"]["a"].get("disabled").is_none());
}

#[test]
fn upsert_add_edit_rename_preserving_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.json");
    // Add creates the file.
    upsert_mcp(
        &path,
        None,
        "fs",
        &McpDraft {
            transport: McpTransport::Stdio,
            command: "npx".into(),
            args: vec!["-y".into(), "fs-server".into()],
            env: vec![("KEY".into(), "secret".into())],
            ..Default::default()
        },
    )
    .unwrap();
    let doc = read_doc(&path).unwrap();
    assert_eq!(doc["mcpServers"]["fs"]["command"], "npx");

    // Duplicate name rejected.
    assert!(upsert_mcp(&path, None, "fs", &McpDraft::default()).is_err());

    // Edit + rename keeps unknown fields (shared/timeout) and enabled flags.
    write(
        &path,
        r#"{"mcpServers":{"fs":{"command":"npx","shared":false,"timeout_secs":60,"enabled":false}}}
"#,
    );
    upsert_mcp(
        &path,
        Some("fs"),
        "fs2",
        &McpDraft {
            transport: McpTransport::Http,
            url: "https://mcp.example".into(),
            headers: vec![("Authorization".into(), "Bearer x".into())],
            ..Default::default()
        },
    )
    .unwrap();
    let doc = read_doc(&path).unwrap();
    let fs2 = &doc["mcpServers"]["fs2"];
    assert!(doc["mcpServers"].get("fs").is_none());
    assert_eq!(fs2["type"], "http");
    assert_eq!(fs2["url"], "https://mcp.example");
    assert_eq!(fs2["shared"], false, "unknown field survives");
    assert_eq!(fs2["timeout_secs"], 60);
    assert_eq!(fs2["enabled"], false, "enabled flag untouched by upsert");
    assert!(fs2.get("command").is_none(), "stdio fields removed");
}

#[test]
fn delete_removes_only_the_target() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.json");
    write(
        &path,
        r#"{"mcpServers":{"a":{"command":"x"},"b":{"command":"y"}}}"#,
    );
    delete_mcp(&path, "a").unwrap();
    let doc = read_doc(&path).unwrap();
    assert!(doc["mcpServers"].get("a").is_none());
    assert!(doc["mcpServers"].get("b").is_some());
    assert!(delete_mcp(&path, "a").is_err());
}

#[test]
fn first_write_backs_up_once() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.json");
    write(&path, r#"{"mcpServers":{"a":{"command":"orig"}}}"#);
    set_mcp_enabled(&path, "a", false).unwrap();
    let mut bak = path.as_os_str().to_os_string();
    bak.push(".bak");
    let bak = PathBuf::from(bak);
    assert!(bak.exists());
    let original = fs::read_to_string(&bak).unwrap();
    assert!(original.contains("orig"));
    // A second write must not overwrite the pristine backup.
    write(&path, r#"{"mcpServers":{"a":{"command":"changed"}}}"#);
    set_mcp_enabled(&path, "a", true).unwrap();
    assert_eq!(fs::read_to_string(&bak).unwrap(), original);
}

#[test]
fn paste_parses_all_three_shapes() {
    // {"mcpServers":{…}}
    let parsed = parse_mcp_paste(
        r#"{"mcpServers":{"a":{"command":"x"},"b":{"url":"https://y"}}}"#,
    )
    .unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].0, "a");
    assert_eq!(parsed[1].1.transport, McpTransport::Http);
    // {"name":{…}}
    let parsed = parse_mcp_paste(r#"{"only":{"command":"z","args":["1"]}}"#).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].0, "only");
    assert_eq!(parsed[0].1.args, ["1"]);
    // single server object → empty name, form supplies it
    let parsed = parse_mcp_paste(r#"{"type":"sse","url":"https://s"}"#).unwrap();
    assert_eq!(parsed.len(), 1);
    assert!(parsed[0].0.is_empty());
    assert_eq!(parsed[0].1.transport, McpTransport::Sse);
    // errors
    assert!(parse_mcp_paste("not json").is_err());
    assert!(parse_mcp_paste("42").is_err());
    assert!(parse_mcp_paste(r#"{"a":1}"#).is_err());
}

#[test]
fn test_output_parsing() {
    let ok = parse_mcp_test_output(
        "Connected to MCP server 'fs' and disconnected cleanly.\nServer: filesystem 1.0\nTools: 2\n  read\n  write\n",
        "",
        true,
    );
    assert!(ok.ok);
    assert_eq!(ok.server.as_deref(), Some("filesystem 1.0"));
    assert_eq!(ok.tools, ["read", "write"]);
    let err = parse_mcp_test_output("", "Error: boom\n", false);
    assert!(!err.ok);
    assert_eq!(err.error.as_deref(), Some("Error: boom"));
}
