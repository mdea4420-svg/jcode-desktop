//! GPUI tests for the Extensions overlay (MCP / Skills / Plugins tabs).
use super::*;
use crate::extensions::ExtensionTab;
use crate::learning;
use jcode_sdk::api::RuntimeCommand;

fn setup<'a>(
    cx: &'a mut gpui::TestAppContext,
) -> (
    gpui::Entity<Panel>,
    &'a mut gpui::VisualTestContext,
    std::sync::mpsc::Receiver<Command>,
) {
    cx.update(crate::input::bind_keys);
    let (bridge, commands) = crate::harness::spawn_recording();
    let (panel, vcx) =
        cx.add_window_view(|_, cx| Panel::new("extensions-test".into(), None, None, bridge, cx));
    vcx.update(|_, cx| Panel::connect_input(&panel, cx));
    (panel, vcx, commands)
}

/// `skill-row-<dir>` selector for the loaded skill with frontmatter `name`.
fn skill_row_selector(panel: &Panel, name: &str) -> &'static str {
    let dir = panel
        .extensions
        .as_ref()
        .unwrap()
        .skills
        .iter()
        .find(|skill| skill.name == name)
        .unwrap_or_else(|| panic!("skill {name} loaded"))
        .dir
        .to_string_lossy()
        .to_string();
    Box::leak(format!("skill-row-{dir}").into_boxed_str())
}

/// A scratch ExtensionPaths with one enabled + one disabled MCP server and
/// two skills, all under a temp dir the caller must keep alive.
fn fixture_paths(root: &std::path::Path) -> ExtensionPaths {
    let jcode_home = root.join("jcode_home");
    let user_home = root.join("home");
    let project = root.join("project");
    let put = |path: std::path::PathBuf, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    };
    put(
        jcode_home.join("mcp.json"),
        r#"{"mcpServers":{"alpha":{"command":"alpha-bin"},"beta":{"command":"beta-bin","disabled":true}}}"#,
    );
    let skill = |dir: std::path::PathBuf, name: &str| {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} helper\n---\n\n# {name}\n"),
        )
        .unwrap();
    };
    skill(jcode_home.join("skills/alpha-skill"), "alpha-skill");
    skill(project.join(".jcode/skills/zebra-skill"), "zebra-skill");
    ExtensionPaths {
        jcode_home,
        user_home,
        project_dir: Some(project),
    }
}

#[gpui::test]
fn slash_mcp_opens_the_mcp_tab(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        assert!(panel.handle_slash_command_for_test("/mcp", cx));
    });
    vcx.run_until_parked();
    panel.read_with(vcx, |panel, _| {
        assert_eq!(panel.extensions.as_ref().unwrap().tab, ExtensionTab::Mcp);
    });
    assert!(vcx.debug_bounds("extensions").is_some());
    assert!(vcx.debug_bounds("mcp-row-filesystem").is_some());
}

#[gpui::test]
fn bare_skills_opens_the_skills_tab(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        assert!(panel.handle_slash_command_for_test("/skills", cx));
    });
    vcx.run_until_parked();
    panel.read_with(vcx, |panel, _| {
        assert_eq!(
            panel.extensions.as_ref().unwrap().tab,
            ExtensionTab::Skills
        );
    });
    let sel = panel.read_with(vcx, |panel, _| skill_row_selector(panel, "commit-style"));
    assert!(vcx.debug_bounds(sel).is_some());
}

#[gpui::test]
fn skills_with_args_still_goes_to_the_runtime(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        assert!(panel.handle_slash_command_for_test("/skills check", cx));
    });
    let command = commands.try_recv().expect("runtime command sent");
    match command {
        Command::RuntimeCommand { command, args, .. } => {
            assert_eq!(command, RuntimeCommand::Skills);
            assert_eq!(args, vec!["check".to_string()]);
        }
        _ => panic!("expected RuntimeCommand"),
    }
    panel.read_with(vcx, |panel, _| assert!(panel.extensions.is_none()));
}

#[gpui::test]
fn slash_extensions_reopens_the_last_tab(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        assert!(panel.handle_slash_command_for_test("/mcp", cx));
        panel.close_extensions(cx);
        assert!(panel.handle_slash_command_for_test("/extensions", cx));
    });
    vcx.run_until_parked();
    panel.read_with(vcx, |panel, _| {
        assert_eq!(panel.extensions.as_ref().unwrap().tab, ExtensionTab::Mcp);
    });
}

#[gpui::test]
fn open_extensions_action_reaches_the_active_panel(cx: &mut gpui::TestAppContext) {
    cx.update(crate::bind_workspace_keys);
    let (bridge, _commands) = crate::harness::spawn_recording();
    let (workspace, vcx) = cx.add_window_view(|_, cx| {
        let mut workspace = crate::workspace::Workspace::for_test(learning::Coach::new(), cx);
        workspace.set_test_bridge(bridge);
        workspace.push_test_panel("extensions-action", cx);
        workspace
    });
    vcx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.focus_active(window, cx));
    });
    vcx.run_until_parked();
    let source = workspace.read_with(vcx, |workspace, _| {
        workspace.test_active_panel().unwrap().entity_id()
    });
    vcx.update(|window, cx| {
        window.dispatch_action(
            Box::new(crate::workspace::OpenExtensions {
                source,
                tab: ExtensionTab::Mcp,
            }),
            cx,
        );
    });
    vcx.run_until_parked();
    let panel =
        workspace.read_with(vcx, |workspace, _| workspace.test_active_panel().unwrap());
    panel.read_with(vcx, |panel, _| {
        assert_eq!(panel.extensions.as_ref().unwrap().tab, ExtensionTab::Mcp);
    });
}

#[gpui::test]
fn mcp_toggle_writes_the_config_file(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let paths = fixture_paths(root.path());
    let mcp_path = paths.jcode_home.join("mcp.json");
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.open_extensions_with_paths(paths, None, ExtensionTab::Mcp, cx);
    });
    vcx.run_until_parked();
    let toggle = vcx
        .debug_bounds("mcp-toggle-alpha")
        .expect("alpha toggle pill");
    vcx.simulate_click(toggle.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert_eq!(
        written["mcpServers"]["alpha"]["disabled"],
        serde_json::Value::Bool(true)
    );
    // The disabled server toggles back on (disabled flag removed).
    let toggle = vcx
        .debug_bounds("mcp-toggle-beta")
        .expect("beta toggle pill");
    vcx.simulate_click(toggle.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(
        written["mcpServers"]["beta"].get("disabled").is_none(),
        "disabled removed: {written}"
    );
}

#[gpui::test]
fn mcp_delete_asks_then_removes(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let paths = fixture_paths(root.path());
    let mcp_path = paths.jcode_home.join("mcp.json");
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.open_extensions_with_paths(paths, None, ExtensionTab::Mcp, cx);
    });
    vcx.run_until_parked();
    let delete = vcx
        .debug_bounds("mcp-delete-alpha")
        .expect("alpha delete pill");
    vcx.simulate_click(delete.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    // Nothing written until the inline confirm.
    let written = std::fs::read_to_string(&mcp_path).unwrap();
    assert!(written.contains("alpha"));
    let confirm = vcx
        .debug_bounds("mcp-confirm-alpha")
        .expect("confirm bar");
    let _ = confirm;
    let yes = vcx
        .debug_bounds("mcp-confirm-delete-alpha")
        .expect("confirm delete pill");
    vcx.simulate_click(yes.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(written["mcpServers"].get("alpha").is_none());
    assert!(written["mcpServers"].get("beta").is_some());
}

#[gpui::test]
fn skill_search_filters_the_list(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.handle_slash_command_for_test("/skills", cx);
    });
    vcx.run_until_parked();
    let (commit_sel, pdf_sel) = panel.read_with(vcx, |panel, _| {
        (
            skill_row_selector(panel, "commit-style"),
            skill_row_selector(panel, "pdf-review"),
        )
    });
    assert!(vcx.debug_bounds(commit_sel).is_some());
    assert!(vcx.debug_bounds(pdf_sel).is_some());
    let search = panel.read_with(vcx, |panel, _| {
        panel.extensions.as_ref().unwrap().search.clone()
    });
    search.update(vcx, |search, cx| search.set_content("commit".into(), cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds(commit_sel).is_some());
    assert!(vcx.debug_bounds(pdf_sel).is_none());
}

#[gpui::test]
fn escape_closes_the_overlay(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.handle_slash_command_for_test("/mcp", cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("extensions").is_some());
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    panel.read_with(vcx, |panel, _| assert!(panel.extensions.is_none()));
    assert!(vcx.debug_bounds("extensions").is_none());
}

#[gpui::test]
fn extensions_and_model_browser_are_mutually_exclusive(cx: &mut gpui::TestAppContext) {
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.available_models = vec!["opus-4-8".into()];
        panel.input.update(cx, |input, cx| {
            input.set_model_routes(
                Vec::new(),
                &[jcode_sdk::ModelRouteInfo {
                    model: "opus-4-8".into(),
                    provider: "openai".into(),
                    api_method: "oauth".into(),
                    available: true,
                    detail: String::new(),
                    usage: None,
                }],
                None,
                cx,
            );
        });
    });
    panel.update_in(vcx, |panel, window, cx| {
        panel.toggle_model_picker(window, cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("model-browser").is_some());
    panel.update(vcx, |panel, cx| {
        panel.handle_slash_command_for_test("/extensions", cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("extensions").is_some());
    assert!(vcx.debug_bounds("model-browser").is_none());
    // Opening the browser again closes the extensions overlay.
    panel.update_in(vcx, |panel, window, cx| {
        panel.toggle_model_picker(window, cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("model-browser").is_some());
    assert!(vcx.debug_bounds("extensions").is_none());
}

/// Long lists must scroll inside the fixed-height dialog: the list stays
/// bounded, the wheel moves it, and a late row reaches the viewport.
#[gpui::test]
fn long_lists_scroll_inside_the_dialog(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let jcode_home = root.path().join("jcode_home");
    std::fs::create_dir_all(jcode_home.join("skills")).unwrap();
    let servers = (0..40)
        .map(|i| format!("\"srv-{i:02}\":{{\"command\":\"bin-{i:02}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        jcode_home.join("mcp.json"),
        format!("{{\"mcpServers\":{{{servers}}}}}"),
    )
    .unwrap();
    for i in 0..40 {
        let dir = jcode_home.join(format!("skills/skill-{i:02}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: skill-{i:02}\ndescription: d{i}\n---\n\n# skill-{i:02}\n"),
        )
        .unwrap();
    }
    let paths = ExtensionPaths {
        jcode_home,
        user_home: root.path().join("home"),
        project_dir: None,
    };
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.open_extensions_with_paths(paths, None, ExtensionTab::Mcp, cx);
    });
    vcx.run_until_parked();
    let wheel = |vcx: &mut gpui::VisualTestContext, center: gpui::Point<gpui::Pixels>| {
        vcx.simulate_event(gpui::ScrollWheelEvent {
            position: center,
            delta: gpui::ScrollDelta::Lines(gpui::point(0., -80.)),
            modifiers: gpui::Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        vcx.run_until_parked();
    };
    let dialog = vcx.debug_bounds("extensions").expect("dialog");
    let list = vcx.debug_bounds("extensions-mcp-list").expect("mcp list");
    assert!(
        list.size.height <= dialog.size.height,
        "the list must stay inside the dialog: {list:?} vs {dialog:?}"
    );
    for _ in 0..3 {
        wheel(vcx, list.center());
    }
    let offset = panel.read_with(vcx, |panel, _| {
        panel.extensions.as_ref().unwrap().scroll.offset().y
    });
    assert!(offset < gpui::px(0.), "the wheel must move the list: {offset}");
    let last = vcx
        .debug_bounds("mcp-row-srv-39")
        .expect("the last row paints");
    assert!(
        last.origin.y < dialog.bottom_right().y,
        "the last row never reaches the viewport: {last:?} vs {dialog:?}"
    );
    // The Skills tab scrolls the same way (switching in place keeps the
    // temp fixture; `/skills` would reopen the screenshot fixture).
    panel.update(vcx, |panel, cx| {
        panel.extensions.as_mut().unwrap().tab = ExtensionTab::Skills;
        cx.notify();
    });
    vcx.run_until_parked();
    let list = vcx
        .debug_bounds("extensions-skills-list")
        .expect("skills list");
    for _ in 0..3 {
        wheel(vcx, list.center());
    }
    let last_sel = panel.read_with(vcx, |panel, _| skill_row_selector(panel, "skill-39"));
    let last = vcx.debug_bounds(last_sel).expect("the last skill row paints");
    assert!(
        last.origin.y < dialog.bottom_right().y,
        "the last skill row never reaches the viewport: {last:?}"
    );
}

#[gpui::test]
fn plugins_tab_creates_filters_and_toggles_plugins_in_isolated_paths(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let paths = fixture_paths(root.path());
    let layout = crate::plugins::layout_for_paths(&paths);
    assert_eq!(layout.plugins_dir, paths.jcode_home.join("plugins"));
    assert_eq!(layout.claude_plugins, Some(paths.user_home.join(".claude/plugins")));
    let default_plugins = crate::plugins::layout().plugins_dir;
    assert_ne!(layout.plugins_dir, default_plugins);
    let (panel, vcx, _commands) = setup(cx);
    panel.update(vcx, |panel, cx| {
        panel.open_extensions_with_paths(paths, None, ExtensionTab::Plugins, cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("extensions-plugins-empty").is_some());

    let click = |vcx: &mut gpui::VisualTestContext, selector| {
        let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
        vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
    };
    let create = |vcx: &mut gpui::VisualTestContext, name, kind| {
        let previously_installed = default_plugins.join(name).exists();
        click(vcx, "plugins-new");
        assert!(vcx.debug_bounds("plugins-form-name").is_some());
        click(vcx, kind);
        vcx.simulate_input(name);
        click(vcx, "plugins-create");
        assert!(vcx.debug_bounds("extensions-plugins-list").is_some());
        assert_eq!(default_plugins.join(name).exists(), previously_installed);
    };
    create(vcx, "test-round3-notes", "plugins-kind-declarative");
    create(vcx, "test-round3-code", "plugins-kind-code");
    let notes = layout.plugins_dir.join("test-round3-notes");
    let code = layout.plugins_dir.join("test-round3-code");
    assert!(notes.join(".claude-plugin/plugin.json").is_file());
    assert!(notes.join("commands/hello.md").is_file());
    assert!(code.join(".claude-plugin/plugin.json").is_file());
    assert!(code.join("index.mjs").is_file());
    assert!(code.join("jcode.d.ts").is_file());
    assert!(vcx.debug_bounds("plugin-open-test-round3-notes").is_some());

    for query in ["NOTES", "what this plugin adds", "/TEST-ROUND3-NOTES:HELLO", "声明式"] {
        vcx.simulate_keystrokes("ctrl-a");
        vcx.simulate_input(query);
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("plugin-row-test-round3-notes").is_some(), "{query}");
        assert!(vcx.debug_bounds("plugin-row-test-round3-code").is_none(), "{query}");
    }
    vcx.simulate_keystrokes("ctrl-a");
    vcx.simulate_input("does-not-exist");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("extensions-plugins-empty").is_some());
    assert!(vcx.debug_bounds("plugin-row-test-round3-notes").is_none());

    vcx.simulate_keystrokes("ctrl-a backspace");
    vcx.run_until_parked();
    click(vcx, "plugin-toggle-test-round3-notes");
    let enabled = || jcode_plugins::State::load(&layout).enabled["test-round3-notes"];
    assert!(layout.state_file().is_file());
    assert!(!enabled());
    click(vcx, "plugin-toggle-test-round3-notes");
    assert!(enabled());

    click(vcx, "plugins-new");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("plugins-form-name").is_none());
    assert!(vcx.debug_bounds("extensions-plugins-list").is_some());
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("extensions").is_none());
}

#[gpui::test]
fn chinese_extensions_keep_stable_tab_selectors_and_localize_plugin_reports(
    cx: &mut gpui::TestAppContext,
) {
    let report = jcode_plugins::SyncReport {
        plugins: 3,
        enabled: 2,
        skills: 1,
        mcp_servers: 4,
        hook_events: vec!["pre_tool".into()],
        warnings: vec!["fixture diagnostic".into()],
        changed: true,
    };
    assert_eq!(
        crate::plugins::report_markdown(&report),
        "Synced 2 enabled of 3 plugins: 1 skills, 4 MCP servers, hooks on pre_tool.\n- warning: fixture diagnostic\n\nNew skills, MCP servers and hooks apply to new sessions (or after the runtime reloads its config)."
    );
    crate::locale::with_zh(|| {
        let root = tempfile::tempdir().unwrap();
        let paths = fixture_paths(root.path());
        let (panel, vcx, _commands) = setup(cx);
        panel.update(vcx, |panel, cx| {
            panel.open_extensions_with_paths(paths, None, ExtensionTab::Mcp, cx);
        });
        vcx.run_until_parked();
        for (selector, tab, list) in [
            ("extensions-tab-skills", ExtensionTab::Skills, "extensions-skills-list"),
            ("extensions-tab-plugins", ExtensionTab::Plugins, "extensions-plugins-list"),
            ("extensions-tab-mcp", ExtensionTab::Mcp, "extensions-mcp-list"),
        ] {
            let bounds = vcx.debug_bounds(selector).expect(selector);
            vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
            panel.read_with(vcx, |panel, _| assert_eq!(panel.extensions.as_ref().unwrap().tab, tab));
            assert!(vcx.debug_bounds(list).is_some());
        }
        let markdown = crate::plugins::report_markdown(&report);
        assert!(markdown.contains("已同步 3 个插件中的 2 个已启用插件"));
        assert!(markdown.contains("1 项技能和 4 个 MCP 服务器"));
        assert!(markdown.contains("警告：fixture diagnostic"));
        assert!(markdown.contains("会在新会话中生效"));
    });
}

#[gpui::test]
fn plugins_tab_bounds_long_text_in_narrow_window_filters_and_trashes_safely(
    cx: &mut gpui::TestAppContext,
) {
    let root = tempfile::tempdir().unwrap();
    let paths = fixture_paths(root.path());
    let layout = crate::plugins::layout_for_paths(&paths);
    let local_dir = layout.plugins_dir.join("ultra-long-local-helper-plugin-name");
    std::fs::create_dir_all(local_dir.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(local_dir.join("commands")).unwrap();
    std::fs::write(
        local_dir.join(".claude-plugin/plugin.json"),
        format!(
            r#"{{"name":"ultra-long-local-helper-plugin-name","version":"1.2.3","description":"C:\\Users\\Administrator\\.claude\\plugins\\cache\\marketplace\\very_long_unbroken_identifier_{}"}}"#,
            "x".repeat(90)
        ),
    )
    .unwrap();
    std::fs::write(
        local_dir.join("commands/very-long-command-identifier-for-overflow-test.md"),
        "Run helper",
    )
    .unwrap();

    let claude_root = paths.user_home.join(".claude/plugins");
    let claude_pkg = claude_root.join("cache/memorax-adapter/0.1.23");
    std::fs::create_dir_all(claude_pkg.join(".claude-plugin")).unwrap();
    std::fs::write(
        claude_pkg.join(".claude-plugin/plugin.json"),
        r#"{"name":"memorax-adapter","version":"0.1.23","description":"Claude adapter plugin"}"#,
    )
    .unwrap();
    std::fs::write(
        claude_root.join("installed_plugins.json"),
        serde_json::json!({
            "plugins": {
                "memorax-adapter@store": [{
                    "installPath": claude_pkg,
                    "version": "0.1.23"
                }]
            }
        })
        .to_string(),
    )
    .unwrap();

    let (panel, vcx, _commands) = setup(cx);
    let window = vcx.update(|window, _| window.window_handle());
    vcx.simulate_window_resize(window, gpui::size(gpui::px(480.), gpui::px(640.)));

    panel.update(vcx, |panel, cx| {
        assert!(panel.handle_slash_command_for_test("/plugins", cx));
        panel.open_extensions_with_paths(paths.clone(), None, ExtensionTab::Plugins, cx);
    });
    vcx.run_until_parked();

    let dialog = vcx.debug_bounds("extensions").expect("extensions dialog");
    for selector in [
        "plugin-row-ultra-long-local-helper-plugin-name",
        "plugin-open-ultra-long-local-helper-plugin-name",
        "plugin-delete-ultra-long-local-helper-plugin-name",
        "plugin-toggle-ultra-long-local-helper-plugin-name",
        "plugin-row-memorax-adapter",
        "plugin-open-memorax-adapter",
        "plugin-toggle-memorax-adapter",
    ] {
        let bounds = vcx.debug_bounds(selector).expect(selector);
        assert!(
            bounds.origin.x >= dialog.origin.x && bounds.bottom_right().x <= dialog.bottom_right().x + gpui::px(1.),
            "{selector} overflowed dialog horizontally: {bounds:?} vs {dialog:?}"
        );
    }
    assert!(
        vcx.debug_bounds("plugin-delete-memorax-adapter").is_none(),
        "external Claude plugins must not expose trash removal"
    );

    let click = |vcx: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
        vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
    };
    click(vcx, "plugins-filter-claude");
    assert!(vcx.debug_bounds("plugin-row-memorax-adapter").is_some());
    assert!(vcx.debug_bounds("plugin-row-ultra-long-local-helper-plugin-name").is_none());
    click(vcx, "plugins-filter-local");
    assert!(vcx.debug_bounds("plugin-row-ultra-long-local-helper-plugin-name").is_some());
    assert!(vcx.debug_bounds("plugin-row-memorax-adapter").is_none());
    click(vcx, "plugins-filter-all");

    click(vcx, "plugin-delete-ultra-long-local-helper-plugin-name");
    assert!(vcx.debug_bounds("plugin-confirm-ultra-long-local-helper-plugin-name").is_some());
    assert!(local_dir.is_dir());
    click(vcx, "plugin-confirm-delete-ultra-long-local-helper-plugin-name");
    assert!(!local_dir.exists());
    let trashed: Vec<_> = std::fs::read_dir(paths.jcode_home.join("trash/plugins"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(trashed.len(), 1);
    assert!(vcx.debug_bounds("plugin-row-ultra-long-local-helper-plugin-name").is_none());
}
