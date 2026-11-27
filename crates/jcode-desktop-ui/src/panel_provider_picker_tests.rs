use super::*;

#[gpui::test]
fn method_pill_switches_between_session_model_routes(cx: &mut gpui::TestAppContext) {
    cx.update(crate::input::bind_keys);
    crate::input::model_favorites::reset();
    const SESSION: &str = "ssh://test-host/session-a";
    let (bridge, commands) = crate::harness::spawn_recording();
    let (panel, vcx) =
        cx.add_window_view(|_, cx| Panel::new(SESSION.into(), None, None, bridge, cx));
    vcx.update(|_, cx| Panel::connect_input(&panel, cx));
    panel.update(vcx, |panel, cx| {
        panel.input.update(cx, |input, _| {
            input.set_dropped_preset_methods(["openai-compatible:magpie".into()].into());
        });
        let route = |model: &str| jcode_sdk::ModelRouteInfo {
            model: model.into(),
            provider: "magpie".into(),
            api_method: "openai-compatible:magpie".into(),
            available: true,
            detail: String::new(),
            usage: None,
        };
        panel.apply(
            &ApiEvent::RuntimeInfo {
                session_id: SESSION.into(),
                provider: Some("magpie".into()),
                model: Some("codex/gpt-6.2-sol".into()),
                reasoning_effort: Some("high".into()),
                auth_method: None,
                routes: vec![route("codex/gpt-6.2-sol"), route("cpr-fast/gpt-6.2-sol")],
            },
            cx,
        );
    });
    vcx.run_until_parked();
    let pill = vcx
        .debug_bounds("panel-login")
        .expect("third composer pill");
    vcx.simulate_click(pill.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("model-browser").is_some());
    assert!(vcx.debug_bounds("accounts-panel").is_none());
    panel.read_with(vcx, |panel, cx| {
        use crate::input::model_menu::{ModelFilter, group_key};
        assert!(panel.login.is_none());
        let current = group_key(
            "magpie:codex/gpt-6.2-sol",
            panel.input.read(cx).model_details(),
        );
        assert_eq!(
            panel.model_browser.as_ref().unwrap().filter,
            ModelFilter::Group(current),
        );
    });
    assert!(commands.try_recv().is_err());

    let other_route = vcx
        .debug_bounds("model-browser-filter-Magpie · cpr-fast")
        .expect("a separate group for the other Magpie channel");
    vcx.simulate_click(other_route.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(commands.try_recv().is_err());
    let model = vcx
        .debug_bounds("model-browser-row-0")
        .expect("model on selected route");
    vcx.simulate_click(model.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(matches!(
        commands.recv_timeout(std::time::Duration::from_millis(100)),
        Ok(Command::SetModel { session_id, model })
            if session_id == SESSION && model == "magpie:cpr-fast/gpt-6.2-sol"
    ));
    assert!(matches!(
        commands.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(vcx.debug_bounds("model-browser").is_none());
    assert!(vcx.debug_bounds("accounts-panel").is_none());

    let current_group = panel.read_with(vcx, |panel, cx| {
        crate::input::model_menu::group_key(
            "magpie:codex/gpt-6.2-sol",
            panel.input.read(cx).model_details(),
        )
    });
    crate::input::model_favorites::toggle_hidden(&current_group);
    let pill = vcx.debug_bounds("panel-login").unwrap();
    vcx.simulate_click(pill.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    panel.read_with(vcx, |panel, _| {
        assert_eq!(
            panel.model_browser.as_ref().unwrap().filter,
            crate::input::model_menu::ModelFilter::All,
        );
    });
    assert!(vcx.debug_bounds("model-browser-filter-Magpie · codex").is_none());
    assert!(vcx.debug_bounds("model-browser-filter-Magpie · cpr-fast").is_some());
    assert!(commands.try_recv().is_err());
    crate::input::model_favorites::reset();
}
