//! Extensions overlay — Plugins tab: discovered fork plugins with the same
//! enable/disable and reload paths `/plugins` uses.

use super::*;
use super::extensions::pill;

impl Panel {
    /// The Plugins tab body.
    pub(super) fn render_extensions_plugins(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let Some(state) = self.extensions.as_ref() else {
            return div().into_any_element();
        };
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        rows.push(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill(
                        "plugins-reload",
                        "plugins-reload".into(),
                        "Reload",
                        false,
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            let report = crate::plugins::sync_now();
                            this.extensions_apply(
                                Ok(crate::plugins::report_markdown(&report)
                                    .lines()
                                    .next()
                                    .unwrap_or_default()
                                    .to_string()),
                                "Reloaded:",
                                cx,
                            );
                        }),
                    ),
                )
                .into_any_element(),
        );
        for plugin in &state.plugins {
            let name = plugin.name.clone();
            let enabled = plugin.enabled;
            let toggle_name = name.clone();
            let detail = format!(
                "{} · {}{}",
                if plugin.code { "code" } else { "declarative" },
                plugin.source,
                if plugin.version.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", plugin.version)
                }
            );
            rows.push(
                div()
                    .id(format!("plugin-row-{}", name.clone()))
                    .debug_selector({
                        let name = name.clone();
                        move || format!("plugin-row-{name}").into()
                    })
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_xl()
                    .bg(theme.HEADER_BG)
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(200.))
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .when(!enabled, |el| el.text_color(theme.TEXT_DIM))
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .child(detail),
                    )
                    .child(
                        pill(
                            format!("plugin-toggle-{}", toggle_name.clone()),
                            format!("plugin-toggle-{toggle_name}"),
                            if enabled { "On" } else { "Off" },
                            enabled,
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                let result = crate::plugins::set_enabled(
                                    &toggle_name,
                                    !enabled,
                                )
                                .map(|report| {
                                    crate::plugins::report_markdown(&report)
                                        .lines()
                                        .next()
                                        .unwrap_or_default()
                                        .to_string()
                                });
                                this.extensions_apply(
                                    result,
                                    if enabled { "Disabled:" } else { "Enabled:" },
                                    cx,
                                );
                            }),
                        ),
                    )
                    .into_any_element(),
            );
        }
        if state.plugins.is_empty() {
            rows.push(
                div()
                    .id("extensions-plugins-empty")
                    .debug_selector(|| "extensions-plugins-empty".into())
                    .flex_none()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(theme.TEXT_DIM)
                    .child("No plugins installed. Run `/plugins` for details.")
                    .into_any_element(),
            );
        }
        div()
            .id("extensions-plugins-list")
            .debug_selector(|| "extensions-plugins-list".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .overflow_y_scroll()
            .track_scroll(&self.extensions.as_ref().unwrap().scroll)
            .children(rows)
            .into_any_element()
    }
}
