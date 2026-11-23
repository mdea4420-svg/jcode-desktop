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
