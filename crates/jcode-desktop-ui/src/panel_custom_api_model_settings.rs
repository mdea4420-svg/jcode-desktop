use super::*;
use crate::custom_providers::{self, MODEL_REASONING_EFFORTS, ModelSettings, ModelSettingsEdit};
use std::collections::{BTreeMap, HashSet};

pub(super) struct ModelSettingsEditor {
    pub(super) target: Option<String>,
    filter: Entity<PromptInput>,
    original: HashMap<String, ModelSettings>,
    forms: HashMap<String, ModelSettingsForm>,
}

struct ModelSettingsForm {
    context_window: Entity<PromptInput>,
    reasoning: Option<bool>,
    effort: Option<String>,
    image_input: Option<bool>,
}

impl CustomApiEditor {
    pub(super) fn model_ids(&self, cx: &App) -> Vec<String> {
        let mut seen = HashSet::new();
        std::iter::once(self.default_model.read(cx).content.trim().to_string())
            .chain(custom_providers::parse_model_list(
                self.extra_models.read(cx).content.as_ref(),
            ))
            .chain(self.fetched_models.iter().cloned())
            .filter(|id| !id.is_empty() && seen.insert(id.clone()))
            .collect()
    }
}

impl ModelSettingsEditor {
    pub(super) fn new(profile: &str, offline: bool, cx: &mut Context<Panel>) -> Self {
        let original = if profile.is_empty() {
            HashMap::new()
        } else if offline {
            custom_providers::offline_model_settings(profile)
        } else {
            custom_providers::provider_model_settings(profile)
        };
        Self {
            original,
            target: None,
            filter: plain_field(tr("Filter models…", "筛选模型…"), "", cx),
            forms: HashMap::new(),
        }
    }

    fn select(&mut self, id: String, cx: &mut Context<Panel>) {
        self.forms.entry(id.clone()).or_insert_with(|| {
            let initial = self.original.get(&id).cloned().unwrap_or_default();
            let context = initial.context_window.map(|tokens| tokens.to_string()).unwrap_or_default();
            ModelSettingsForm {
                context_window: plain_field(tr("Inherit", "继承"), &context, cx),
                reasoning: initial.reasoning,
                effort: initial.reasoning_effort,
                image_input: initial.image_input,
            }
        });
        self.target = Some(id);
    }

    fn has_effective_settings(&self, id: &str, cx: &App) -> bool {
        if let Some(form) = self.forms.get(id) {
            let context = form.context_window.read(cx).content.trim();
            return context.parse::<usize>().is_ok_and(|tokens| {
                tokens > 0 && i64::try_from(tokens).is_ok()
                    && context.bytes().all(|byte| byte.is_ascii_digit())
            }) || form.reasoning.is_some() || form.effort.is_some() || form.image_input.is_some();
        }
        self.original.get(id).is_some_and(|model| {
            model.context_window.is_some() || model.reasoning.is_some()
                || model.reasoning_effort.is_some() || model.image_input.is_some()
        })
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
            let original_context = initial.context_window.map(|tokens| tokens.to_string());
            let context = form.context_window.read(cx).content.trim();
            let context_window = if context == original_context.as_deref().unwrap_or("") {
                None
            } else if context.is_empty() {
                Some(None)
            } else {
                let tokens = context
                    .parse::<usize>()
                    .ok()
                    .filter(|tokens| {
                        *tokens > 0 && i64::try_from(*tokens).is_ok()
                            && context.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .ok_or_else(|| {
                        let error = tr(
                            "Context window must be a positive whole number of tokens.",
                            "上下文窗口必须填写正整数 token 数。",
                        );
                        (id.clone(), format!("{id} · {error}"))
                    })?;
                (Some(tokens) != initial.context_window).then_some(Some(tokens))
            };
            let edit = ModelSettingsEdit {
                context_window,
                reasoning: (form.reasoning != initial.reasoning).then_some(form.reasoning),
                reasoning_effort: (form.effort != initial.reasoning_effort)
                    .then(|| form.effort.clone()),
                image_input: (form.image_input != initial.image_input).then_some(form.image_input),
            };
            if edit != ModelSettingsEdit::default() {
                edits.insert(id.clone(), edit);
            }
        }
        Ok(edits)
    }
}

impl Panel {
    pub(super) fn render_custom_model_settings(
        &self,
        editor: &CustomApiEditor,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let models = editor.model_ids(cx);
        if models.is_empty() {
            return None;
        }
        let theme = Theme::global();
        let settings = &editor.model_settings;
        let current = settings.target.as_ref().filter(|id| models.contains(id));
        let filter = settings.filter.read(cx).content.trim().to_lowercase();
        let targets = models.iter().filter(|model| model.to_lowercase().contains(&filter)).map(|model| {
            let target = model.clone();
            let selector = format!("custom-api-settings-target-{target}");
            action_pill(selector.clone(), selector, "", false)
                .max_w_full()
                .when(current == Some(model), |el| {
                    el.bg(theme.ACCENT_DIM).text_color(theme.ACCENT)
                })
                .when(settings.has_effective_settings(model, cx), |el| el.border_color(theme.ACCENT))
                .child(div().min_w_0().truncate().child(model.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(editor) = this.login.as_mut().and_then(|state| state.custom_api.as_mut()) else {
                        return;
                    };
                    if !editor.saving {
                        editor.model_settings.select(target.clone(), cx);
                        cx.notify();
                    }
                }))
        });
        let inherit = tr("Inherit", "继承");
        let form = current.and_then(|id| settings.forms.get(id));
        Some(div()
            .debug_selector(|| "custom-api-model-settings".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD)
                    .child(tr("Model parameters", "模型参数")),
            )
            .when(models.len() > 6 || !settings.filter.read(cx).content.is_empty(), |el| {
                el.child(div().id("custom-api-settings-filter")
                    .debug_selector(|| "custom-api-settings-filter".into())
                    .child(settings.filter.clone()))
            })
            .child(
                div().id("custom-api-settings-targets").max_h(px(88.)).overflow_y_scroll()
                    .child(div().flex().flex_wrap().gap_1p5().children(targets)),
            )
            .when_some(form, |el, form| {
                el.child(field_block(
                    tr("Context window (tokens)", "上下文窗口（token）"),
                    tr(
                        "Leave blank to inherit. A synced model catalog may take precedence.",
                        "留空则继承。同步的模型目录若提供上下文长度，会优先使用目录值。",
                    ),
                    form.context_window.clone().into_any_element(),
                ))
                .child(div().flex().flex_wrap().gap_1p5().children([
                    ("custom-api-context-preset-128k", "128K", "128000"),
                    ("custom-api-context-preset-200k", "200K", "200000"),
                    ("custom-api-context-preset-272k", "272K", "272000"),
                    ("custom-api-context-preset-1m", "1M", "1000000"),
                    ("custom-api-context-preset-inherit", tr("Inherit", "恢复继承"), ""),
                ].into_iter().map(|(id, label, value)| {
                    let field = form.context_window.clone();
                    toggle_pill(id, label, field.read(cx).content.trim() == value,
                        cx.listener(move |this, _, _, cx| {
                            if this.login.as_ref().and_then(|state| state.custom_api.as_ref())
                                .is_some_and(|editor| !editor.saving)
                            {
                                field.update(cx, |input, cx| input.set_content(value.into(), cx));
                                cx.notify();
                            }
                        }))
                })))
                .child(setting_choices(
                    tr("Reasoning", "推理"),
                    [
                        ("custom-api-reasoning-inherit", inherit, None),
                        ("custom-api-reasoning-on", tr("On", "开启"), Some(true)),
                        ("custom-api-reasoning-off", tr("Off", "关闭"), Some(false)),
                    ],
                    &form.reasoning,
                    |form, value| form.reasoning = value,
                    cx,
                ))
                .child(setting_choices(
                    tr("Default reasoning effort", "默认推理强度"),
                    [
                        ("custom-api-effort-inherit", inherit, None),
                        effort_choice("minimal", tr("Minimal", "最低")),
                        effort_choice("low", tr("Low", "低")),
                        effort_choice("medium", tr("Medium", "中")),
                        effort_choice("high", tr("High", "高")),
                        effort_choice("xhigh", tr("Extra high", "很高")),
                        effort_choice("max", tr("Maximum", "最高")),
                    ],
                    &form.effort,
                    |form, value| form.effort = value,
                    cx,
                ))
                .when_some(
                    form.effort.as_deref().filter(|effort| !MODEL_REASONING_EFFORTS.contains(effort)),
                    |el, effort| {
                        el.child(div().text_size(px(11.)).text_color(theme.TEXT_DIM).child(format!(
                            "{} · {effort}", tr("Saved value", "已保存值")
                        )))
                    },
                )
                .child(setting_choices(
                    tr("Input", "输入类型"),
                    [
                        ("custom-api-input-inherit", inherit, None),
                        ("custom-api-input-text", tr("Text", "文本"), Some(false)),
                        ("custom-api-input-image", tr("Text + image", "文本 + 图片"), Some(true)),
                    ],
                    &form.image_input,
                    |form, value| form.image_input = value,
                    cx,
                ))
            })
            .into_any_element())
    }
}

fn effort_choice(
    effort: &'static str,
    label: &'static str,
) -> (&'static str, &'static str, Option<String>) {
    (effort, label, Some(effort.to_string()))
}

fn setting_choices<T: Clone + PartialEq + 'static>(
    label: &'static str,
    choices: impl IntoIterator<Item = (&'static str, &'static str, T)>,
    selected: &T,
    set: fn(&mut ModelSettingsForm, T),
    cx: &mut Context<Panel>,
) -> gpui::Div {
    let pills = choices.into_iter().map(|(id, label, value)| {
        toggle_pill(id, label, selected == &value, cx.listener(move |this, _, _, cx| {
            let Some(editor) = this.login.as_mut().and_then(|state| state.custom_api.as_mut()) else {
                return;
            };
            let settings = &mut editor.model_settings;
            if !editor.saving
                && let Some(id) = settings.target.as_ref()
                && let Some(form) = settings.forms.get_mut(id)
            {
                set(form, value.clone());
                cx.notify();
            }
        }))
    });
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(label))
        .child(div().flex().flex_wrap().gap_1p5().children(pills))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn custom_model_settings_keep_separate_drafts_and_retain_key_on_validation(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::input::bind_keys);
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(900.), px(1000.)));
        panel.update(vcx, |panel, cx| {
            panel.open_custom_api_editor(custom_providers::offline_summaries().pop(), cx);
        });
        vcx.run_until_parked();
        let codex = "codex/gpt-6.2-sol";
        let fast = "cpr-fast/gpt-6.2-sol";
        let target = vcx.debug_bounds("custom-api-settings-target-cpr-fast/gpt-6.2-sol").unwrap();
        vcx.simulate_click(target.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("custom-api-reasoning-on").is_some());
        panel.update(vcx, |panel, cx| {
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            assert_eq!(editor.model_settings.target.as_deref(), Some(fast));
            assert_eq!(editor.default_model.read(cx).content.as_ref(), codex);
            editor.model_settings.forms[fast].context_window.update(cx, |input, cx| {
                input.set_content("196607".into(), cx);
            });
            editor.model_settings.select(codex.into(), cx);
            let settings = &editor.model_settings.forms[codex];
            assert_eq!(settings.context_window.read(cx).content.as_ref(), "272000");
            assert_eq!(settings.effort.as_deref(), Some("high"));
            assert_eq!(settings.reasoning, Some(true));
            assert_eq!(settings.image_input, Some(true));
            settings.context_window.update(cx, |input, cx| input.set_content(String::new(), cx));
            let edits = editor.model_settings.pending_edits(&editor.model_ids(cx), cx).unwrap();
            assert_eq!(edits[codex].context_window, Some(None));
            assert_eq!(edits[fast].context_window, Some(Some(196_607)));

            editor.model_settings.forms[fast].context_window.update(cx, |input, cx| {
                input.set_content("0".into(), cx);
            });
            editor.api_key.update(cx, |input, cx| input.set_secret("fixture-key".into(), cx));
            panel.submit_custom_api(cx);
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert_eq!(editor.model_settings.target.as_deref(), Some(fast));
            assert!(!editor.api_key.read(cx).content_empty());
            assert!(editor.notice.as_ref().is_some_and(|(notice, error)| *error && notice.contains(fast)));
        });
    }

    #[gpui::test]
    fn custom_model_settings_filter_preserves_drafts_and_context_presets(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(1000.), px(1200.)));
        let model = "fixture/model";
        panel.update(vcx, |panel, cx| {
            panel.open_custom_api_editor(custom_providers::offline_summaries().pop(), cx);
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            editor.fetched_models.extend([model, "extra-1", "extra-2", "extra-3", "extra-4", "extra-5", "extra-6"].map(str::to_owned));
            assert!(!editor.model_settings.has_effective_settings(model, cx));
            assert!(editor.model_settings.has_effective_settings("codex/gpt-6.2-sol", cx));
            editor.model_settings.filter.update(cx, |input, cx| input.set_content("FIXTURE/".into(), cx));
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("custom-api-settings-filter").is_some());
        let target = vcx.debug_bounds("custom-api-settings-target-fixture/model").unwrap();
        vcx.simulate_click(target.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let preset = vcx.debug_bounds("custom-api-context-preset-200k").unwrap();
        vcx.simulate_click(preset.center(), gpui::Modifiers::default());
        panel.update(vcx, |panel, cx| {
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            assert!(editor.model_settings.has_effective_settings(model, cx));
            editor.model_settings.filter.update(cx, |input, cx| input.set_content("CODEX".into(), cx));
            let edits = editor.model_settings.pending_edits(&editor.model_ids(cx), cx).unwrap();
            assert_eq!(edits[model].context_window, Some(Some(200_000)));
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("custom-api-settings-target-fixture/model").is_none());
        let inherit = vcx.debug_bounds("custom-api-context-preset-inherit").unwrap();
        vcx.simulate_click(inherit.center(), gpui::Modifiers::default());
        panel.read_with(vcx, |panel, cx| {
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert!(!editor.model_settings.has_effective_settings(model, cx));
            assert!(editor.model_settings.pending_edits(&editor.model_ids(cx), cx).unwrap().is_empty());
        });
    }
}
