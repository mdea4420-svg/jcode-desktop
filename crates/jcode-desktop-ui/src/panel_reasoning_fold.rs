//! Folding for settled thinking (`/thinking collapsed`, the default).
//!
//! While a turn runs its thoughts stay expanded; when the turn finishes they
//! fold to a compact pill with a one-line preview. The pill expands a thought
//! in place and an expanded one offers "Hide thinking". Fold state shares
//! `expanded_tools`, which already feeds the transcript cache key and the
//! selection document. `alt-t` or `/thinking toggle` toggles every settled
//! thought at once.
use super::*;
use crate::locale::{is_zh, tr};

gpui::actions!(reasoning_fold, [ToggleAllThinking]);

pub(crate) fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([gpui::KeyBinding::new(
        "alt-t",
        ToggleAllThinking,
        Some("ChatPanel"),
    )]);
}

const PREVIEW_CHARS: usize = 120;

/// `expanded_tools` key for a thought the user expanded by hand.
pub(super) fn expansion_key(index: usize) -> String {
    format!("reasoning:{index}")
}

/// `expanded_tools` key for a thought the user folded by hand. An explicit
/// collapse wins over the in-progress turn.
fn hide_key(index: usize) -> String {
    format!("reasoning-hide:{index}")
}

/// Whether a reasoning row at `index` is shown folded. `turn_start` is the
/// user prompt index that began the in-progress turn, if any; settled rows
/// after it stay expanded until the turn finishes. Live rows use indices at
/// the top of the range and never fold.
pub(super) fn is_folded(
    display: &crate::config::DisplayConfig,
    index: usize,
    expanded: &HashSet<String>,
    turn_start: Option<usize>,
) -> bool {
    if display.thinking_display != crate::config::ThinkingDisplay::Collapsed
        || index >= usize::MAX / 2
    {
        return false;
    }
    if expanded.contains(&hide_key(index)) {
        return true;
    }
    if expanded.contains(&expansion_key(index)) {
        return false;
    }
    turn_start.is_none_or(|start| index <= start)
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

fn line_word(count: usize) -> &'static str {
    if count == 1 { "line" } else { "lines" }
}

impl Panel {
    /// The settled thought at `index`, kept expanded until folded again.
    fn expand_reasoning(&mut self, index: usize, cx: &mut Context<Self>) {
        self.expanded_tools.remove(&hide_key(index));
        self.expanded_tools.insert(expansion_key(index));
        self.transcript_measurements.dirty = true;
        cx.notify();
    }

    /// Fold the settled thought at `index`, even while its turn still runs.
    fn collapse_reasoning(&mut self, index: usize, cx: &mut Context<Self>) {
        self.expanded_tools.remove(&expansion_key(index));
        self.expanded_tools.insert(hide_key(index));
        self.transcript_measurements.dirty = true;
        cx.notify();
    }

    /// The user prompt index that began the in-progress turn, if any. Rows
    /// after it keep their thinking expanded until the turn settles. Only
    /// genuine activity counts: a reattached or reconnecting session holds
    /// transport statuses (`connected`, `lost: …`) that `is_busy` treats as
    /// busy even though no generation is running.
    pub(super) fn current_turn_start(&self) -> Option<usize> {
        if !self.activity_active() {
            return None;
        }
        self.items
            .iter()
            .rposition(|item| matches!(item, Item::User(_)))
    }

    /// `alt-t` / `/thinking toggle`: expand every settled thought, or fold
    /// them all back when none is folded. Returns true when it expanded.
    pub(super) fn toggle_all_thinking(&mut self, cx: &mut Context<Self>) -> bool {
        let display = crate::config::display();
        let turn_start = self.current_turn_start();
        let indices: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                matches!(item, Item::Reasoning(_)).then_some(index)
            })
            .collect();
        let expand = indices
            .iter()
            .any(|&index| is_folded(&display, index, &self.expanded_tools, turn_start));
        for index in indices {
            if expand {
                self.expanded_tools.remove(&hide_key(index));
                self.expanded_tools.insert(expansion_key(index));
            } else {
                self.expanded_tools.remove(&expansion_key(index));
                self.expanded_tools.insert(hide_key(index));
            }
        }
        self.transcript_measurements.dirty = true;
        cx.notify();
        expand
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
                panel.expand_reasoning(index, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if is_zh() {
                        format!("思考 · {lines} 行")
                    } else {
                        format!("Thinking · {lines} {}", line_word(lines))
                    }),
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
                panel.collapse_reasoning(index, cx);
            }))
            .child(tr("Hide thinking", "收起思考"))
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
        assert!(is_folded(&collapsed(), 3, &expanded, None));
        assert!(!is_folded(&collapsed(), usize::MAX - 1, &expanded, None));
        expanded.insert(expansion_key(3));
        assert!(!is_folded(&collapsed(), 3, &expanded, None));
        let full = crate::config::DisplayConfig {
            thinking_display: crate::config::ThinkingDisplay::Full,
            ..Default::default()
        };
        assert!(!is_folded(&full, 4, &HashSet::new(), None));
    }

    #[test]
    fn the_running_turn_keeps_its_thoughts_expanded() {
        let expanded = HashSet::new();
        // Turn started at index 0: its settled thought stays expanded, an
        // older turn's thought stays folded.
        assert!(!is_folded(&collapsed(), 3, &expanded, Some(0)));
        assert!(is_folded(&collapsed(), 3, &expanded, Some(5)));
        // An explicit collapse folds even inside the running turn.
        let mut expanded = HashSet::new();
        expanded.insert(hide_key(3));
        assert!(is_folded(&collapsed(), 3, &expanded, Some(0)));
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


    fn session_status(status: &str) -> ApiEvent {
        ApiEvent::SessionStatus {
            session_id: "fold-test".into(),
            status: status.into(),
            pending_soft_interrupts: None,
        }
    }

    fn fold_fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<Panel>, &mut gpui::VisualTestContext) {
        cx.update(|cx| {
            crate::config::set_display("thinking_display", "collapsed").unwrap();
        });
        cx.add_window_view(|_, cx| {
            Panel::new("fold-test".into(), None, None, crate::harness::spawn_inert(), cx)
        })
    }

    #[gpui::test]
    fn current_turn_thinking_folds_when_the_turn_finishes(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = fold_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = vec![
                Item::User("first prompt".into()),
                Item::Reasoning("thinking for the current turn".into()),
            ];
            panel.apply(&session_status("processing"), cx);
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("reasoning-fold-1").is_none(),
            "the running turn's thought stays expanded"
        );
        assert!(
            vcx.debug_bounds("reasoning-unfold-1").is_some(),
            "Hide thinking shows on the current-turn thought"
        );
        panel.update(vcx, |panel, cx| panel.apply(&session_status("idle"), cx));
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("reasoning-fold-1").is_some(),
            "the finished turn's thought folds without a click"
        );
    }

    #[gpui::test]
    fn older_turns_fold_while_a_new_turn_runs(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = fold_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = vec![
                Item::User("older prompt".into()),
                Item::Reasoning("older thinking".into()),
                Item::Assistant("older answer".into()),
                Item::User("new prompt".into()),
                Item::Reasoning("current thinking".into()),
            ];
            panel.apply(&session_status("processing"), cx);
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_some());
        assert!(vcx.debug_bounds("reasoning-fold-4").is_none());
    }

    #[gpui::test]
    fn fold_pill_and_hide_control_roundtrip(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = fold_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = vec![
                Item::User("prompt".into()),
                Item::Reasoning("settled thinking".into()),
            ];
            cx.notify();
        });
        vcx.run_until_parked();
        let pill = vcx.debug_bounds("reasoning-fold-1").unwrap();
        vcx.simulate_click(pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_none());
        let hide = vcx.debug_bounds("reasoning-unfold-1").unwrap();
        vcx.simulate_click(hide.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_some());

        // The same controls work mid-turn, including an explicit collapse.
        panel.update(vcx, |panel, cx| panel.apply(&session_status("processing"), cx));
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("reasoning-fold-1").is_some(),
            "an explicit collapse persists into the next turn"
        );
        let pill = vcx.debug_bounds("reasoning-fold-1").unwrap();
        vcx.simulate_click(pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let hide = vcx.debug_bounds("reasoning-unfold-1").unwrap();
        vcx.simulate_click(hide.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("reasoning-fold-1").is_some(),
            "Hide thinking folds a current-turn thought"
        );
    }

    /// `alt-t` toggles whole-turn steps now; the per-thought toggle lives on
    /// `/thinking toggle`.
    #[gpui::test]
    fn thinking_toggle_expands_then_folds_every_thought(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = fold_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = vec![
                Item::User("first".into()),
                Item::Reasoning("older thinking".into()),
                Item::Assistant("answer".into()),
                Item::User("second".into()),
                Item::Reasoning("newer thinking".into()),
            ];
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_some());
        assert!(vcx.debug_bounds("reasoning-fold-4").is_some());
        panel.update(vcx, |panel, cx| {
            assert!(panel.handle_slash_command_for_test("/thinking toggle", cx));
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_none());
        assert!(vcx.debug_bounds("reasoning-fold-4").is_none());
        panel.update(vcx, |panel, cx| {
            assert!(panel.handle_slash_command_for_test("/thinking toggle", cx));
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_some());
        assert!(vcx.debug_bounds("reasoning-fold-4").is_some());
    }

    #[test]
    fn preview_skips_markup_and_blank_lines() {
        assert_eq!(preview("\n\n## Plan the **fix**\n- step"), "Plan the fix");
        let long = "x".repeat(PREVIEW_CHARS + 5);
        assert_eq!(preview(&long).chars().count(), PREVIEW_CHARS + 1);
    }
}
