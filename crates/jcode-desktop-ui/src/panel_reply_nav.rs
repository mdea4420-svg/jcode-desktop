//! Reply navigator: every turn of the conversation in one list, with
//! keyboard stepping between turns.
//!
//! - Alt+O (or the corner pill) toggles the list. Clicking a turn scrolls the
//!   transcript to its prompt.
//! - Alt+Up / Alt+Down jump to the previous or next prompt. Past the last
//!   prompt, Alt+Down returns to the live end.
use super::*;

gpui::actions!(reply_nav, [ToggleReplyNavigator, PreviousPrompt, NextPrompt]);

const PREVIEW_CHARS: usize = 90;
const LIST_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/></svg>"#;

pub(crate) fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        gpui::KeyBinding::new("alt-o", ToggleReplyNavigator, Some("ChatPanel")),
        gpui::KeyBinding::new("alt-up", PreviousPrompt, Some("ChatPanel")),
        gpui::KeyBinding::new("alt-down", NextPrompt, Some("ChatPanel")),
    ]);
}

/// One navigable turn: its list row, its prompt and the first reply line.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Turn {
    pub row: usize,
    pub prompt: String,
    pub reply: Option<String>,
}

fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(['#', '-', '*', '>', ' '])
                .trim()
        })
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .replace("**", "")
        .replace('`', "");
    let mut short: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        short.push('…');
    }
    short
}

/// Turns from the cached prompt rows. A reply is the first assistant text
/// after the prompt and before the next user message.
pub(super) fn turns(items: &[Item], prompt_rows: &[(usize, usize)]) -> Vec<Turn> {
    prompt_rows
        .iter()
        .filter_map(|&(row, index)| {
            let Item::User(text) = items.get(index)? else {
                return None;
            };
            let (visible, _) = super::voice::tag::strip(text);
            let reply = items[index + 1..]
                .iter()
                .take_while(|item| !matches!(item, Item::User(_)))
                .find_map(|item| match item {
                    Item::Assistant(text) if !text.trim().is_empty() => Some(one_line(text)),
                    _ => None,
                });
            Some(Turn {
                row,
                prompt: one_line(&visible),
                reply,
            })
        })
        .collect()
}

/// Prompt row before (`step < 0`) or after the current top row.
pub(super) fn step_target(
    prompt_rows: &[usize],
    top_row: usize,
    top_offset: f32,
    step: i32,
) -> Option<usize> {
    if step < 0 {
        prompt_rows
            .iter()
            .rev()
            .find(|&&row| row < top_row || (row == top_row && top_offset > 1.0))
            .copied()
    } else {
        prompt_rows.iter().find(|&&row| row > top_row).copied()
    }
}

impl Panel {
    pub(super) fn toggle_reply_navigator(&mut self, cx: &mut Context<Self>) {
        self.reply_nav_open = !self.reply_nav_open;
        cx.notify();
    }

    fn jump_to_transcript_row(&mut self, row: usize, cx: &mut Context<Self>) {
        self.cancel_transcript_momentum();
        self.release_startup_preview();
        self.stick_to_bottom = false;
        self.transcript_list.scroll_to(gpui::ListOffset {
            item_ix: row,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    pub(super) fn step_prompt(&mut self, step: i32, cx: &mut Context<Self>) {
        let rows: Vec<usize> = self
            .transcript_derived(cx)
            .prompt_rows
            .iter()
            .map(|(row, _)| *row)
            .collect();
        let top = self.transcript_list.logical_scroll_top();
        // While following the tail the logical top is an end sentinel, so
        // "previous" lands on the latest prompt.
        let (top_row, top_offset) = if self.stick_to_bottom {
            (usize::MAX, 0.0)
        } else {
            (top.item_ix, f32::from(top.offset_in_item))
        };
        match step_target(&rows, top_row, top_offset, step) {
            Some(row) => self.jump_to_transcript_row(row, cx),
            None if step > 0 => self.jump_to_latest(cx),
            None => {}
        }
    }

    /// Corner pill that opens the navigator. Hidden for empty sessions.
    pub(super) fn render_reply_nav_toggle(
        &self,
        prompt_rows: &[(usize, usize)],
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if prompt_rows.len() < 2 || self.transcript_only {
            return None;
        }
        let theme = Theme::global();
        Some(
            div()
                .id("reply-nav-toggle")
                .debug_selector(|| "reply-nav-toggle".into())
                .absolute()
                .top(px(6.))
                .right(px(14.))
                .flex()
                .items_center()
                .gap_1()
                .px(px(8.))
                .h(px(22.))
                .rounded_full()
                .bg(theme.HEADER_BG)
                .border_1()
                .border_color(theme.PANEL_BORDER)
                .text_size(px(10.5))
                .text_color(if self.reply_nav_open {
                    theme.TEXT
                } else {
                    theme.TEXT_DIM
                })
                .cursor_pointer()
                .hover(|el| el.text_color(theme.TEXT))
                .occlude()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|panel, _, _, cx| {
                        cx.stop_propagation();
                        panel.toggle_reply_navigator(cx);
                    }),
                )
                .child(
                    gpui::svg()
                        .data(LIST_ICON)
                        .size(px(11.))
                        .flex_none()
                        .text_color(theme.TEXT_DIM),
                )
                .child(format!("{} turns", prompt_rows.len()))
                .into_any_element(),
        )
    }

    /// The open navigator: one pill row per turn, newest last.
    pub(super) fn render_reply_navigator(
        &self,
        prompt_rows: &[(usize, usize)],
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.reply_nav_open || prompt_rows.is_empty() {
            return None;
        }
        let theme = Theme::global();
        let turns = turns(&self.items, prompt_rows);
        let rows = turns.into_iter().enumerate().map(|(number, turn)| {
            let row = turn.row;
            div()
                .id(("reply-nav-row", row))
                .debug_selector(move || format!("reply-nav-row-{number}").into())
                .flex()
                .items_start()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_full()
                .cursor_pointer()
                .hover(|el| el.bg(theme.INPUT_BG))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |panel, _, _, cx| {
                        cx.stop_propagation();
                        panel.reply_nav_open = false;
                        panel.jump_to_transcript_row(row, cx);
                    }),
                )
                .child(
                    div()
                        .flex_none()
                        .min_w(px(20.))
                        .h(px(18.))
                        .px(px(5.))
                        .rounded_full()
                        .bg(theme.INLINE_CODE_BG)
                        .font_family(theme.FONT_MONO)
                        .text_size(px(10.))
                        .line_height(px(18.))
                        .text_center()
                        .text_color(theme.TEXT_DIM)
                        .child((number + 1).to_string()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .text_color(theme.TEXT_USER)
                                .child(turn.prompt),
                        )
                        .children(turn.reply.map(|reply| {
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(11.))
                                .text_color(theme.TEXT_FAINT)
                                .child(reply)
                        })),
                )
        });
        Some(
            div()
                .id("reply-navigator")
                .debug_selector(|| "reply-navigator".into())
                .absolute()
                .top(px(34.))
                .right(px(14.))
                .w(px(340.))
                .max_h(px(440.))
                .flex()
                .flex_col()
                .p_2()
                .gap_1()
                .rounded_xl()
                .bg(theme.HEADER_BG)
                .border_1()
                .border_color(theme.PANEL_BORDER)
                .occlude()
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .flex()
                        .justify_between()
                        .text_size(px(10.5))
                        .text_color(theme.TEXT_FAINT)
                        .child("Jump to a turn")
                        .child("Alt+↑ / Alt+↓"),
                )
                .child(
                    div()
                        .id("reply-navigator-list")
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_pair_prompts_with_their_first_reply() {
        let items = vec![
            Item::User("## First question".into()),
            Item::Reasoning("thinking".into()),
            Item::Assistant("**Answer** one\nmore".into()),
            Item::User("Second".into()),
            Item::User("Third".into()),
            Item::Assistant("Reply three".into()),
        ];
        let turns = turns(&items, &[(0, 0), (2, 3), (3, 4)]);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].prompt, "First question");
        assert_eq!(turns[0].reply.as_deref(), Some("Answer one"));
        assert_eq!(turns[1].reply, None);
        assert_eq!(turns[2].row, 3);
        assert_eq!(turns[2].reply.as_deref(), Some("Reply three"));
    }

    #[gpui::test]
    fn navigator_rows_and_steps_scroll_to_prompts(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = cx.add_window_view(|_, cx| {
            Panel::new(
                "reply-nav".into(),
                None,
                None,
                crate::harness::spawn_inert(),
                cx,
            )
        });
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(700.), px(600.)));
        panel.update(vcx, |panel, cx| {
            for turn in 0..3 {
                panel.items.push(Item::User(format!("Prompt {turn}")));
                for line in 0..30 {
                    panel
                        .items
                        .push(Item::Assistant(format!("Turn {turn} response {line}")));
                }
            }
            panel.stick_to_bottom = true;
            cx.notify();
        });
        vcx.run_until_parked();
        let prompt_rows: Vec<usize> = panel.update(vcx, |panel, cx| {
            panel
                .transcript_derived(cx)
                .prompt_rows
                .iter()
                .map(|(row, _)| *row)
                .collect()
        });
        assert_eq!(prompt_rows.len(), 3);

        let toggle = vcx.debug_bounds("reply-nav-toggle").expect("toggle shown");
        vcx.simulate_click(toggle.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("reply-navigator").is_some());
        let first = vcx.debug_bounds("reply-nav-row-0").expect("first turn row");
        vcx.simulate_click(first.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert!(!panel.reply_nav_open);
            assert!(!panel.stick_to_bottom);
            assert_eq!(panel.transcript_list.logical_scroll_top().item_ix, prompt_rows[0]);
        });

        panel.update(vcx, |panel, cx| panel.step_prompt(1, cx));
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.transcript_list.logical_scroll_top().item_ix, prompt_rows[1]);
        });
        panel.update(vcx, |panel, cx| panel.step_prompt(-1, cx));
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.transcript_list.logical_scroll_top().item_ix, prompt_rows[0]);
        });
    }

    #[test]
    fn stepping_moves_between_prompt_rows() {
        let rows = [2, 10, 30];
        assert_eq!(step_target(&rows, 12, 0.0, -1), Some(10));
        assert_eq!(step_target(&rows, 10, 0.0, -1), Some(2));
        assert_eq!(step_target(&rows, 10, 40.0, -1), Some(10));
        assert_eq!(step_target(&rows, 10, 0.0, 1), Some(30));
        assert_eq!(step_target(&rows, 30, 0.0, 1), None);
        assert_eq!(step_target(&rows, usize::MAX, 0.0, -1), Some(30));
        assert_eq!(step_target(&rows, 1, 0.0, -1), None);
    }
}
