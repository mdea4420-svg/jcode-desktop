//! Opt-in folding for settled thinking (`/thinking collapsed`).
//!
//! The default presentation keeps every thought inline. In collapsed mode a
//! settled thought becomes a compact pill with a one-line preview, and the
//! pill toggles the full text. Live thinking is never folded, so the current
//! turn still streams in full. Fold state shares `expanded_tools`, which
//! already feeds the transcript cache key and the selection document.
use super::*;

const PREVIEW_CHARS: usize = 120;

pub(super) fn expansion_key(index: usize) -> String {
    format!("reasoning:{index}")
}

/// Whether a reasoning row at `index` is shown folded. Only settled rows
/// fold. Live rows use indices at the top of the range.
pub(super) fn is_folded(
    display: &crate::config::DisplayConfig,
    index: usize,
    expanded: &HashSet<String>,
) -> bool {
    display.thinking_display == crate::config::ThinkingDisplay::Collapsed
        && index < usize::MAX / 2
        && !expanded.contains(&expansion_key(index))
}

/// First meaningful line, without Markdown heading and list markers.
pub(super) fn preview(text: &str) -> String {
    let line = text
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(['#', '-', '*', '>', ' '])
                .trim()
        })
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut preview: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    preview.replace("**", "").replace('`', "")
}

impl Panel {
    fn toggle_reasoning_fold(&mut self, index: usize, cx: &mut Context<Self>) {
        let key = expansion_key(index);
        if !self.expanded_tools.remove(&key) {
            self.expanded_tools.insert(key);
        }
        self.transcript_measurements.dirty = true;
        cx.notify();
    }

    /// Folded pill. Clicking it expands the thought in place.
    pub(super) fn render_reasoning_fold(
        &self,
        index: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
        div()
            .id(("reasoning-fold", index))
            .debug_selector(move || format!("reasoning-fold-{index}").into())
            .flex()
            .items_center()
            .gap_2()
            .max_w_full()
            .min_w_0()
            .px(px(10.))
            .h(px(24.))
            .rounded_full()
            .bg(theme.INLINE_CODE_BG)
            .font_family(theme.FONT_AI)
            .text_size(px(11.5))
            .text_color(theme.REASONING)
            .cursor_pointer()
            .hover(|style| style.text_color(theme.TEXT))
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.toggle_reasoning_fold(index, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("Thinking · {lines} lines")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(theme.TEXT_FAINT)
                    .child(preview(text)),
            )
            .into_any_element()
    }

    /// Compact pill under an expanded thought that folds it again.
    pub(super) fn render_reasoning_unfold_control(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        div()
            .id(("reasoning-unfold", index))
            .debug_selector(move || format!("reasoning-unfold-{index}").into())
            .mt_1()
            .flex_none()
            .self_start()
            .px(px(8.))
            .h(px(20.))
            .flex()
            .items_center()
            .rounded_full()
            .bg(theme.INLINE_CODE_BG)
            .text_size(px(10.5))
            .text_color(theme.TEXT_DIM)
            .cursor_pointer()
            .hover(|style| style.text_color(theme.TEXT))
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.toggle_reasoning_fold(index, cx);
            }))
            .child("Hide thinking")
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collapsed() -> crate::config::DisplayConfig {
        crate::config::DisplayConfig {
            thinking_display: crate::config::ThinkingDisplay::Collapsed,
            ..Default::default()
        }
    }

    #[test]
    fn only_settled_rows_fold_until_expanded() {
        let mut expanded = HashSet::new();
        assert!(is_folded(&collapsed(), 3, &expanded));
        assert!(!is_folded(&collapsed(), usize::MAX - 1, &expanded));
        expanded.insert(expansion_key(3));
        assert!(!is_folded(&collapsed(), 3, &expanded));
        assert!(!is_folded(&Default::default(), 4, &HashSet::new()));
    }

    #[test]
    fn folded_thoughts_leave_the_selection_document() {
        let item = Item::Reasoning("a long private thought".into());
        let segments = |expanded| {
            super::super::text_document::segments_with_display(
                &item,
                2,
                false,
                expanded,
                &collapsed(),
            )
        };
        assert!(segments(false).is_empty());
        assert!(!segments(true).is_empty());
    }

    #[test]
    fn preview_skips_markup_and_blank_lines() {
        assert_eq!(preview("\n\n## Plan the **fix**\n- step"), "Plan the fix");
        let long = "x".repeat(PREVIEW_CHARS + 5);
        assert_eq!(preview(&long).chars().count(), PREVIEW_CHARS + 1);
    }
}
