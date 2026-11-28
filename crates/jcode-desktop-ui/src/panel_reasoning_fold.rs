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

/// Whether a settled reasoning row at `index` is followed by non-reasoning
/// output (such as tool calls, an assistant reply, or live streaming answer
/// text), meaning this thinking block has already concluded.
pub(super) fn followed_by_output(
    items: &[Item],
    index: usize,
    has_streaming_text: bool,
) -> bool {
    if index >= usize::MAX / 2 {
        return false;
    }
    has_streaming_text
        || items
            .get(index + 1..)
            .is_some_and(|rest| rest.iter().any(|item| !matches!(item, Item::Reasoning(_))))
}

/// Whether a reasoning row at `index` is shown folded. `turn_start` is the
/// user prompt index that began the in-progress turn, if any; an active
/// trailing thought stays expanded while the model is still thinking, and
/// automatically folds once answer text or tool work begins (or when the turn
/// finishes). Live rows use indices at the top of the range and never fold.
pub(super) fn is_folded(
    display: &crate::config::DisplayConfig,
    index: usize,
    expanded: &HashSet<String>,
    turn_start: Option<usize>,
) -> bool {
    is_folded_with_progress(display, index, expanded, turn_start, false)
}

pub(super) fn is_folded_with_progress(
    display: &crate::config::DisplayConfig,
    index: usize,
    expanded: &HashSet<String>,
    turn_start: Option<usize>,
    concluded_by_output: bool,
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
    concluded_by_output || turn_start.is_none_or(|start| index <= start)
}

/// Split an encoded assistant `HistoryMessage` payload into ordered
/// `Item::Reasoning` and `Item::Assistant` segments so restored/reloaded
/// conversations fold their thinking identically to live turns. Fenced code
/// blocks containing transport markup are preserved verbatim inside the
/// assistant segment.
pub(super) fn split_assistant_history_segments(content: &str) -> Vec<Item> {
    if content.trim().is_empty() {
        return Vec::new();
    }
    if !content
        .lines()
        .any(|line| jcode_render_core::reasoning_line_content(line.trim_start()).is_some())
    {
        return vec![Item::Assistant(content.to_owned())];
    }

    let mut out = Vec::new();
    let mut lines = content.lines().peekable();
    let mut prose = String::new();
    let mut fence_marker: Option<char> = None;

    let flush_prose = |prose: &mut String, out: &mut Vec<Item>| {
        let trimmed_end = prose.trim_end();
        if !trimmed_end.trim().is_empty() {
            out.push(Item::Assistant(trimmed_end.trim_start_matches('\n').to_owned()));
        }
        prose.clear();
    };

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence_marker {
            prose.push_str(line);
            prose.push('\n');
            let t = line.trim();
            if t.len() >= 3 && t.chars().all(|c| c == marker) {
                fence_marker = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence_marker = trimmed.chars().next();
            prose.push_str(line);
            prose.push('\n');
            continue;
        }
        if let Some(mut thought) = jcode_render_core::reasoning_line_content(trimmed) {
            flush_prose(&mut prose, &mut out);
            loop {
                let mut ahead = lines.clone();
                let mut blank_lines = 0usize;
                while ahead.peek().is_some_and(|next| next.trim().is_empty()) {
                    ahead.next();
                    blank_lines += 1;
                }
                let Some(next_thought) = ahead
                    .next()
                    .map(str::trim_start)
                    .and_then(jcode_render_core::reasoning_line_content)
                else {
                    break;
                };
                for _ in 0..blank_lines {
                    lines.next();
                    thought.push('\n');
                }
                lines.next();
                thought.push('\n');
                thought.push_str(&next_thought);
            }
            if !thought.trim().is_empty() {
                append_reasoning(&mut out, thought);
            }
        } else {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    flush_prose(&mut prose, &mut out);
    out
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
        let expand = indices.iter().any(|&index| {
            let concluded =
                followed_by_output(&self.items, index, !self.streaming_text.is_empty());
            is_folded_with_progress(
                &display,
                index,
                &self.expanded_tools,
                turn_start,
                concluded,
            )
        });
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

    fn reasoning_meta_label(&self, index: usize, lines: usize, live: bool) -> String {
        let lines = lines.max(1);
        let duration = (index < usize::MAX / 2)
            .then(|| self.reasoning_durations.get(&index).copied())
            .flatten()
            .filter(|&secs| secs >= 0.2)
            .map(|secs| format_tool_elapsed(Duration::from_secs_f64(secs)));
        if live {
            return if is_zh() {
                format!("正在思考… · {lines} 行")
            } else {
                format!("Thinking… · {lines} {}", line_word(lines))
            };
        }
        match (is_zh(), duration) {
            (true, Some(dur)) => format!("已思考 {dur} · {lines} 行"),
            (true, None) => format!("思考过程 · {lines} 行"),
            (false, Some(dur)) => format!("Thought for {dur} · {lines} {}", line_word(lines)),
            (false, None) => format!("Thinking · {lines} {}", line_word(lines)),
        }
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
        let summary = preview(text);
        div()
            .id(("reasoning-fold", index))
            .debug_selector(move || format!("reasoning-fold-{index}").into())
            .flex()
            .items_center()
            .gap_2()
            .max_w_full()
            .min_w_0()
            .px(px(11.))
            .h(px(26.))
            .rounded_full()
            .border_1()
            .border_color(theme.PANEL_BORDER)
            .bg(theme.INLINE_CODE_BG)
            .font_family(theme.FONT_UI)
            .text_size(px(11.5))
            .text_color(theme.REASONING)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(theme.ACCENT_DIM)
                    .border_color(theme.ACCENT)
                    .text_color(theme.TEXT)
            })
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.expand_reasoning(index, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.5))
                    .text_color(theme.TEXT_DIM)
                    .child("▸"),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.reasoning_meta_label(index, lines, false)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(theme.TEXT_FAINT)
                    .child(summary),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.5))
                    .text_color(theme.TEXT_DIM)
                    .child(tr("Expand", "展开")),
            )
            .into_any_element()
    }

    /// Top in-place header bar above an expanded thought so clicking the exact
    /// same spot folds/unfolds without hunting at the bottom of a long block.
    pub(super) fn render_reasoning_expanded_header(
        &self,
        index: usize,
        lines: usize,
        live: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let meta = self.reasoning_meta_label(index, lines, live);
        if live {
            return div()
                .flex_none()
                .self_start()
                .px(px(10.))
                .h(px(22.))
                .flex()
                .items_center()
                .gap_1p5()
                .rounded_full()
                .bg(theme.INLINE_CODE_BG)
                .font_family(theme.FONT_UI)
                .text_size(px(11.))
                .text_color(theme.ACCENT)
                .child("▾")
                .child(meta)
                .into_any_element();
        }
        div()
            .id(("reasoning-unfold", index))
            .debug_selector(move || format!("reasoning-unfold-{index}").into())
            .flex_none()
            .w_full()
            .min_w_0()
            .px(px(11.))
            .h(px(26.))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded_full()
            .border_1()
            .border_color(theme.PANEL_BORDER)
            .bg(theme.INLINE_CODE_BG)
            .font_family(theme.FONT_UI)
            .text_size(px(11.5))
            .text_color(theme.REASONING)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(theme.ACCENT_DIM)
                    .border_color(theme.ACCENT)
                    .text_color(theme.TEXT)
            })
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.collapse_reasoning(index, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.5))
                            .text_color(theme.TEXT_DIM)
                            .child("▾"),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(meta),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.5))
                    .text_color(theme.TEXT_DIM)
                    .child(tr("Hide thinking", "收起思考")),
            )
            .into_any_element()
    }

    /// Secondary bottom fold pill shown on long expanded thoughts (`>= 8` lines)
    /// so the user can collapse right from the bottom without scrolling back up.
    pub(super) fn render_reasoning_bottom_fold(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        div()
            .id(("reasoning-unfold-bottom", index))
            .debug_selector(move || format!("reasoning-unfold-bottom-{index}").into())
            .mt_1()
            .flex_none()
            .self_start()
            .px(px(9.))
            .h(px(21.))
            .flex()
            .items_center()
            .gap_1()
            .rounded_full()
            .border_1()
            .border_color(theme.PANEL_BORDER)
            .bg(theme.INLINE_CODE_BG)
            .font_family(theme.FONT_UI)
            .text_size(px(10.5))
            .text_color(theme.TEXT_DIM)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(theme.ACCENT_DIM)
                    .border_color(theme.ACCENT)
                    .text_color(theme.TEXT)
            })
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.collapse_reasoning(index, cx);
            }))
            .child("↑")
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

    #[test]
    fn restored_assistant_messages_split_encoded_thinking_into_foldable_items() {
        let encoded = format!(
            "{}{}{}\nFinal **answer**.\n```text\n{}```",
            jcode_render_core::reasoning_line_markup("## Step 1"),
            jcode_render_core::reasoning_line_markup(""),
            jcode_render_core::reasoning_line_markup("Inspect the fold state."),
            jcode_render_core::reasoning_line_markup("Inside code fence stays literal."),
        );
        let split = split_assistant_history_segments(&encoded);
        assert_eq!(split.len(), 2);
        assert_eq!(
            split[0],
            Item::Reasoning("## Step 1\n\nInspect the fold state.".into())
        );
        assert!(matches!(&split[1], Item::Assistant(body) if body.starts_with("Final **answer**.") && body.contains("Inside code fence stays literal.")));
    }

    #[gpui::test]
    fn settled_thinking_auto_folds_as_soon_as_reply_begins_and_keeps_top_and_bottom_toggles(
        cx: &mut gpui::TestAppContext,
    ) {
        let (panel, vcx) = fold_fixture(cx);
        let long_thought = (1..=10)
            .map(|n| format!("Thought line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        panel.update(vcx, |panel, cx| {
            panel.items.push(Item::User("Explain this".into()));
            panel.apply(&session_status("processing"), cx);
            panel.apply(
                &ApiEvent::ReasoningDelta {
                    session_id: "fold-test".into(),
                    text: long_thought.clone(),
                },
                cx,
            );
            panel.apply(
                &ApiEvent::ReasoningDone {
                    session_id: "fold-test".into(),
                    duration_secs: Some(3.4),
                },
                cx,
            );
        });
        vcx.run_until_parked();
        // Before answer starts streaming, active thought stays open with top + bottom controls.
        assert!(vcx.debug_bounds("reasoning-fold-1").is_none());
        assert!(vcx.debug_bounds("reasoning-unfold-1").is_some());
        assert!(vcx.debug_bounds("reasoning-unfold-bottom-1").is_some());

        // Once the reply begins streaming, the concluded thought auto-folds out of the way.
        panel.update(vcx, |panel, cx| {
            panel.apply(
                &ApiEvent::TextDelta {
                    message_id: None,
                    session_id: "fold-test".into(),
                    text: "Here is the final answer.".into(),
                },
                cx,
            );
        });
        vcx.run_until_parked();
        let pill = vcx
            .debug_bounds("reasoning-fold-1")
            .expect("settled thought auto-folds when answer streaming begins");
        // Clicking the pill mid-stream expands it with the in-place top bar and bottom fold pill.
        vcx.simulate_click(pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-unfold-1").is_some());
        let bottom = vcx
            .debug_bounds("reasoning-unfold-bottom-1")
            .expect("long expanded thought shows bottom fold pill");
        vcx.simulate_click(bottom.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reasoning-fold-1").is_some());
    }
}
