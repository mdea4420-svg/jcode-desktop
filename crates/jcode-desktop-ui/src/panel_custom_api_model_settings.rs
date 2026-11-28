use super::*;
use crate::custom_providers::{MODEL_REASONING_EFFORTS, ModelSettings, ModelSettingsEdit};
use std::collections::BTreeMap;

pub(super) struct ModelSettingsEditor {
    pub(super) target: Option<String>,
    original: HashMap<String, ModelSettings>,
    forms: HashMap<String, ModelSettingsForm>,
}

struct ModelSettingsForm {
    alias: Entity<PromptInput>,
    context: Entity<PromptInput>,
    output: Entity<PromptInput>,
    settings: ModelSettings,
}

impl ModelSettingsEditor {
    pub(super) fn new(profile: &str, offline: bool, _cx: &mut Context<Panel>) -> Self {
        let original = if profile.is_empty() {
            HashMap::new()
        } else if offline {
            custom_providers::offline_model_settings(profile)
        } else {
            custom_providers::provider_model_settings(profile)
        };
        Self {
            target: None,
            original,
            forms: HashMap::new(),
        }
    }

    pub(super) fn cloned_from(profile: &str, offline: bool, cx: &mut Context<Panel>) -> Self {
        let source = if profile.is_empty() {
            HashMap::new()
        } else if offline {
            custom_providers::offline_model_settings(profile)
        } else {
            custom_providers::provider_model_settings(profile)
        };
        let forms = source
            .into_iter()
            .map(|(id, settings)| {
                (
                    id,
                    ModelSettingsForm {
                        alias: plain_field(
                            tr("e.g. fast", "例如 fast"),
                            settings.alias.as_deref().unwrap_or(""),
                            cx,
                        ),
                        context: token_input(settings.context_window, cx),
                        output: token_input(settings.max_output_tokens, cx),
                        settings,
                    },
                )
            })
            .collect();
        Self {
            target: None,
            original: HashMap::new(),
            forms,
        }
    }

    pub(super) fn toggle(&mut self, id: &str, cx: &mut Context<Panel>) {
        if self.target.as_deref() == Some(id) {
            self.target = None;
            return;
        }
        self.forms.entry(id.to_owned()).or_insert_with(|| {
            let settings = self.original.get(id).cloned().unwrap_or_default();
            ModelSettingsForm {
                alias: plain_field(
                    tr("e.g. fast", "例如 fast"),
                    settings.alias.as_deref().unwrap_or(""),
                    cx,
                ),
                context: token_input(settings.context_window, cx),
                output: token_input(settings.max_output_tokens, cx),
                settings,
            }
        });
        self.target = Some(id.to_owned());
    }

    pub(super) fn alias<'a>(&'a self, id: &str, cx: &'a App) -> Option<&'a str> {
        match self.forms.get(id) {
            Some(form) => input_value(&form.alias, cx),
            None => self
                .original
                .get(id)
                .and_then(|settings| settings.alias.as_deref()),
        }
    }

    pub(super) fn summary(
        &self,
        id: &str,
        catalog: Option<&CatalogModelEntry>,
        cx: &App,
    ) -> String {
        let (context, output, settings) = match self.forms.get(id) {
            Some(form) => (
                tokens(&form.context, cx).ok().flatten(),
                tokens(&form.output, cx).ok().flatten(),
                Some(&form.settings),
            ),
            None => {
                let settings = self.original.get(id);
                (
                    settings.and_then(|s| s.context_window),
                    settings.and_then(|s| s.max_output_tokens),
                    settings,
                )
            }
        };
        let context = context.or_else(|| catalog.and_then(|entry| entry.context_window));
        let output = output.or_else(|| catalog.and_then(|entry| entry.max_output_tokens));
        let mut parts: Vec<_> = [context, output]
            .into_iter()
            .flatten()
            .map(compact_tokens)
            .collect();
        if settings.is_some_and(|s| s.reasoning == Some(false)) {
            parts.push(tr("Off", "关闭").into());
        } else if let Some(effort) = settings
            .and_then(|s| s.reasoning_effort.as_deref())
            .or_else(|| catalog.and_then(|entry| entry.default_reasoning_effort.as_deref()))
        {
            parts.push(effort_label(effort).into());
        }
        parts.join(" · ")
    }

    pub(super) fn pending_edits(
        &self,
        models: &[String],
        cx: &App,
    ) -> Result<BTreeMap<String, ModelSettingsEdit>, (String, String)> {
        let mut edits = BTreeMap::new();
        for id in models {
            let Some(form) = self.forms.get(id) else {
                continue;
            };
            let initial = self.original.get(id).cloned().unwrap_or_default();
            let context = tokens(&form.context, cx).map_err(|()| {
                (
                    id.clone(),
                    format!(
                        "{id} · {}",
                        tr(
                            "Context window must be a positive whole number of tokens.",
                            "上下文窗口必须填写正整数 token 数。"
                        )
                    ),
                )
            })?;
            let output = tokens(&form.output, cx).map_err(|()| {
                (
                    id.clone(),
                    format!(
                        "{id} · {}",
                        tr(
                            "Max output must be a positive whole number of tokens.",
                            "最大输出必须填写正整数 token 数。"
                        )
                    ),
                )
            })?;
            let value = &form.settings;
            let edit = ModelSettingsEdit {
                alias: changed(
                    &initial.alias,
                    &input_value(&form.alias, cx).map(str::to_owned),
                ),
                context_window: changed(&initial.context_window, &context),
                max_output_tokens: changed(&initial.max_output_tokens, &output),
                reasoning: changed(&initial.reasoning, &value.reasoning),
                reasoning_levels: changed(&initial.reasoning_levels, &value.reasoning_levels),
                reasoning_effort: changed(&initial.reasoning_effort, &value.reasoning_effort),
                image_input: changed(&initial.image_input, &value.image_input),
                pdf_input: changed(&initial.pdf_input, &value.pdf_input),
                subagent_schedulable: changed(
                    &initial.subagent_schedulable,
                    &value.subagent_schedulable,
                ),
            };
            if edit != ModelSettingsEdit::default() {
                edits.insert(id.clone(), edit);
            }
        }
        Ok(edits)
    }
}

fn changed<T: Clone + PartialEq>(initial: &T, current: &T) -> Option<T> {
    (initial != current).then(|| current.clone())
}

fn input_value<'a>(field: &Entity<PromptInput>, cx: &'a App) -> Option<&'a str> {
    let value = field.read(cx).content.trim();
    (!value.is_empty()).then_some(value)
}

fn tokens(field: &Entity<PromptInput>, cx: &App) -> Result<Option<usize>, ()> {
    input_value(field, cx)
        .map(|value| {
            value
                .parse::<usize>()
                .ok()
                .filter(|n| {
                    *n > 0
                        && i64::try_from(*n).is_ok()
                        && value.bytes().all(|byte| byte.is_ascii_digit())
                })
                .ok_or(())
        })
        .transpose()
}

fn token_input(value: Option<usize>, cx: &mut Context<Panel>) -> Entity<PromptInput> {
    let value = value.map(|n| n.to_string()).unwrap_or_default();
    plain_field(tr("Inherit", "继承"), &value, cx)
}

fn effort_label(effort: &str) -> &str {
    match effort {
        "minimal" => tr("Minimal", "最低"),
        "low" => tr("Low", "低"),
        "medium" => tr("Medium", "中"),
        "high" => tr("High", "高"),
        "xhigh" => tr("Extra high", "很高"),
        "max" => tr("Maximum", "最高"),
        _ => effort,
    }
}

const CONTEXT_PRESETS: &[(&str, &str, &str)] = &[
    ("128k", "128K", "128000"),
    ("200k", "200K", "200000"),
    ("256k", "256K", "256000"),
    ("512k", "512K", "512000"),
    ("1m", "1M", "1000000"),
];
const OUTPUT_PRESETS: &[(&str, &str, &str)] = &[
    ("4k", "4K", "4000"),
    ("8k", "8K", "8000"),
    ("16k", "16K", "16000"),
    ("32k", "32K", "32000"),
    ("64k", "64K", "64000"),
    ("128k", "128K", "128000"),
];

impl Panel {
    pub(super) fn render_custom_model_settings(
        &self,
        editor: &CustomApiEditor,
        id: &str,
        _catalog: Option<&CatalogModelEntry>,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if editor.model_settings.target.as_deref() != Some(id) {
            return None;
        }
        let form = editor.model_settings.forms.get(id)?;
        let value = &form.settings;
        let inherit = tr("Inherit", "继承");
        let mut levels = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(toggle_pill(
                "custom-api-reasoning-inherit",
                tr("Follow gateway", "跟随网关"),
                value.reasoning.is_none() && value.reasoning_levels.is_none(),
                setting_action(id, cx, |s| {
                    s.reasoning = None;
                    s.reasoning_levels = None;
                }),
            ))
            .child(toggle_pill(
                "custom-api-reasoning-off",
                tr("Off", "关闭"),
                value.reasoning == Some(false),
                setting_action(id, cx, |s| {
                    s.reasoning = Some(false);
                    s.reasoning_levels = None;
                }),
            ));
        let mut effort = div().flex().flex_wrap().gap_1().child(toggle_pill(
            "custom-api-effort-inherit",
            inherit,
            value.reasoning_effort.is_none(),
            setting_action(id, cx, |s| s.reasoning_effort = None),
        ));
        for &level in MODEL_REASONING_EFFORTS {
            let active = value.reasoning != Some(false)
                && value
                    .reasoning_levels
                    .as_ref()
                    .is_some_and(|levels| levels.iter().any(|item| item == level));
            levels = levels.child(toggle_pill(
                format!("custom-api-reasoning-level-{level}"),
                effort_label(level),
                active,
                setting_action(id, cx, move |s| {
                    let configured = s.reasoning_levels.get_or_insert_with(Vec::new);
                    if let Some(index) = configured.iter().position(|item| item == level) {
                        configured.remove(index);
                    } else {
                        configured.push(level.to_owned());
                        configured.sort_by_key(|item| {
                            MODEL_REASONING_EFFORTS
                                .iter()
                                .position(|candidate| *candidate == item.as_str())
                                .unwrap_or(usize::MAX)
                        });
                    }
                    if configured.is_empty() {
                        s.reasoning = None;
                        s.reasoning_levels = None;
                    } else {
                        s.reasoning = Some(true);
                    }
                }),
            ));
            effort = effort.child(toggle_pill(
                format!("custom-api-effort-{level}"),
                effort_label(level),
                value.reasoning_effort.as_deref() == Some(level),
                setting_action(id, cx, move |s| s.reasoning_effort = Some(level.to_owned())),
            ));
        }
        Some(
            div()
                .debug_selector(|| "custom-api-model-settings".into())
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .child(field_block(
                    tr("Alias", "别名"),
                    div()
                        .debug_selector(|| "custom-api-model-alias".into())
                        .child(form.alias.clone()),
                ))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(token_controls(
                            "context",
                            tr("Context window", "上下文窗口"),
                            &form.context,
                            CONTEXT_PRESETS,
                            cx,
                        ))
                        .child(token_controls(
                            "output",
                            tr("Max output", "最大输出"),
                            &form.output,
                            OUTPUT_PRESETS,
                            cx,
                        )),
                )
                .child(field_block(
                    tr("Supported reasoning levels", "支持的思考等级"),
                    levels,
                ))
                .child(field_block(
                    tr("Default reasoning level", "默认思考等级"),
                    effort,
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(capability(
                            "image",
                            tr("Images", "图片"),
                            value.image_input,
                            id,
                            |s, v| s.image_input = v,
                            cx,
                        ))
                        .child(capability(
                            "pdf",
                            "PDF",
                            value.pdf_input,
                            id,
                            |s, v| s.pdf_input = v,
                            cx,
                        ))
                        .child(capability(
                            "subagent",
                            tr("Subagent scheduling", "可供子代理调度"),
                            value.subagent_schedulable,
                            id,
                            |s, v| s.subagent_schedulable = v,
                            cx,
                        )),
                )
                .into_any_element(),
        )
    }
}

fn token_controls(
    prefix: &'static str,
    label: &'static str,
    field: &Entity<PromptInput>,
    presets: &'static [(&'static str, &'static str, &'static str)],
    cx: &Context<Panel>,
) -> gpui::Div {
    let selector = format!("custom-api-model-{prefix}");
    let pills = presets
        .iter()
        .copied()
        .chain([("inherit", tr("Restore inherited", "恢复继承"), "")])
        .map(|(key, label, value)| {
            let active = field.read(cx).content.trim() == value;
            let field = field.clone();
            toggle_pill(
                format!("custom-api-{prefix}-preset-{key}"),
                label,
                active,
                cx.listener(move |panel, _, _, cx| {
                    if panel.custom_api_editor_mut().is_some() {
                        field.update(cx, |input, cx| input.set_content(value.into(), cx));
                        cx.notify();
                    }
                }),
            )
        });
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(field_block(
            label,
            div()
                .debug_selector(move || selector.clone())
                .child(field.clone()),
        ))
        .child(div().flex().flex_wrap().gap_1().children(pills))
}

fn capability(
    prefix: &'static str,
    label: &'static str,
    selected: Option<bool>,
    id: &str,
    set: fn(&mut ModelSettings, Option<bool>),
    cx: &Context<Panel>,
) -> gpui::Div {
    let inherited = if prefix == "subagent" {
        tr("Inherit (on)", "继承（开启）")
    } else {
        tr("Inherit", "继承")
    };
    let choices = [
        ("inherit", inherited, None),
        ("on", tr("On", "开启"), Some(true)),
        ("off", tr("Off", "关闭"), Some(false)),
    ];
    field_block(
        label,
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .children(choices.map(|(key, label, value)| {
                toggle_pill(
                    format!("custom-api-{prefix}-{key}"),
                    label,
                    selected == value,
                    setting_action(id, cx, move |s| set(s, value)),
                )
            })),
    )
}

fn setting_action(
    id: &str,
    cx: &Context<Panel>,
    update: impl Fn(&mut ModelSettings) + 'static,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let id = id.to_owned();
    edit_action(cx, move |editor| {
        if let Some(form) = editor.model_settings.forms.get_mut(&id) {
            update(&mut form.settings);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn custom_api_model_settings_keep_separate_drafts_and_retain_key_on_validation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        panel.update(vcx, |panel, cx| {
            panel.open_custom_api_editor(custom_providers::offline_summaries().pop(), cx);
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            let codex = "codex/gpt-6.2-sol";
            let fast = "cpr-fast/gpt-6.2-sol";
            editor.model_settings.toggle(fast, cx);
            assert_eq!(editor.default_model.as_deref(), Some(codex));
            let fast_context = editor.model_settings.forms[fast].context.clone();
            fast_context.update(cx, |input, cx| input.set_content("196607".into(), cx));
            editor.model_settings.toggle(codex, cx);
            let codex_form = &editor.model_settings.forms[codex];
            assert_eq!(tokens(&codex_form.context, cx), Ok(Some(272_000)));
            assert_eq!(
                codex_form.settings.reasoning_effort.as_deref(),
                Some("high")
            );
            assert_eq!(codex_form.settings.reasoning, Some(true));
            assert_eq!(codex_form.settings.image_input, Some(true));
            codex_form
                .context
                .update(cx, |input, cx| input.set_content(String::new(), cx));
            let edits = editor
                .model_settings
                .pending_edits(&editor.selected_models, cx)
                .unwrap();
            assert_eq!(edits[codex].context_window, Some(None));
            assert_eq!(edits[fast].context_window, Some(Some(196_607)));

            fast_context.update(cx, |input, cx| input.set_content("0".into(), cx));
            editor
                .api_key
                .update(cx, |input, cx| input.set_secret("fixture-key".into(), cx));
            panel.submit_custom_api(cx);
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert_eq!(editor.model_settings.target.as_deref(), Some(fast));
            assert!(!editor.api_key.read(cx).content_empty());
            assert!(
                editor
                    .notice
                    .as_ref()
                    .is_some_and(|(notice, error)| *error && notice.contains(fast))
            );
        });
    }

    #[gpui::test]
    fn custom_api_model_settings_filter_preserves_drafts_and_context_presets(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::input::bind_keys);
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        let window = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(window, gpui::size(px(1200.), px(1100.)));
        let model = "fixture/model";
        panel.update(vcx, |panel, cx| {
            panel.open_custom_api_editor(None, cx);
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            editor.add_model(model.into());
            editor.add_model("other/model".into());
            editor
                .settings_filter
                .update(cx, |input, cx| input.set_content("FIXTURE/".into(), cx));
        });
        vcx.run_until_parked();
        let click = |cx: &mut gpui::VisualTestContext, id: &'static str| {
            let bounds = cx
                .debug_bounds(id)
                .unwrap_or_else(|| panic!("missing {id}"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        };
        click(vcx, "custom-api-settings-target-fixture/model");
        click(vcx, "custom-api-context-preset-200k");
        panel.update(vcx, |panel, cx| {
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            editor
                .settings_filter
                .update(cx, |input, cx| input.set_content("other".into(), cx));
            let edits = editor
                .model_settings
                .pending_edits(&editor.selected_models, cx)
                .unwrap();
            assert_eq!(edits[model].context_window, Some(Some(200_000)));
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("custom-api-settings-target-fixture/model")
                .is_none()
        );
        panel.update(vcx, |panel, cx| {
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            editor
                .settings_filter
                .update(cx, |input, cx| input.set_content(String::new(), cx));
        });
        vcx.run_until_parked();
        click(vcx, "custom-api-context-preset-inherit");
        panel.read_with(vcx, |panel, cx| {
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert!(
                editor
                    .model_settings
                    .pending_edits(&editor.selected_models, cx)
                    .unwrap()
                    .is_empty()
            );
        });
    }
}
