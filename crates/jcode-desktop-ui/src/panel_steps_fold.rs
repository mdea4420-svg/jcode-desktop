//! Folding a finished turn's work steps into one pill (`/steps collapsed`,
//! the default).
//!
//! A turn runs from an `Item::User` to the next prompt (or the end). Its
//! trailing run of assistant text, errors, stats footers, stop notices, and
//! assistant-produced images is the *answer* and stays visible; everything
//! between the prompt and the answer — thoughts, tool calls and their
//! results or images, intermediate assistant text, notices such as cache
//! misses, background-task rows, applet rows — is the *steps* block. While
//! the turn runs its steps stay expanded; once it finishes they collapse to
//! a single pill ("Worked for 2m · 6 tool calls · 4 thoughts"). The pill
//! expands the block in place, an expanded block offers a compact
//! "Hide steps" control, and `alt-t` or `/steps toggle` toggles every turn
//! at once. Fold state shares `expanded_tools`, which already feeds the
//! transcript cache key and the selection document.
//!
//! Variant inventory — how each `Item` classifies when a turn is split:
//! - `User`: opens a turn; the prompt row is never folded.
//! - `Image`: `UserInput` attachments ride with the prompt; `ToolResult`
//!   images stay a step (they belong to their call); `Other` images are
//!   steps mid-turn but count as answer when they trail the turn (final
//!   images the assistant produced as its answer stay visible).
//! - `Assistant`: a step between prompt and answer, visible inside the
//!   trailing answer run.
//! - `Reasoning`, `Tool`, `BackgroundTask`, `Todos`: always steps — the
//!   work process, including still-running tasks.
//! - `Error`, `Stopped`, `ResponseStats`, `CacheMiss`: steps mid-turn,
//!   visible when they close the turn (an errored turn folds its steps and
//!   keeps the trailing error; a cache-miss notice that lands after the
//!   answer must not pull the answer into the fold).
//! - `Applet`, `StepsFold`: derived rows with no settled index; an applet
//!   folds when it is emitted inside a steps block.
use super::*;
use crate::locale::{is_zh, tr};

/// What the steps-control row of one turn renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepsFold {
    /// `Item::User` index that opened the turn; the `expanded_tools` key stem.
    pub turn: usize,
    /// Set when the row is the folded summary pill, clear when it is the
    /// "Hide steps" control above an expanded block.
    pub folded: bool,
    pub tools: usize,
    pub failed: usize,
    pub thoughts: usize,
    /// Steps that are neither tool calls nor thoughts.
    pub other: usize,
    pub duration_secs: Option<f64>,
}

/// `expanded_tools` key for a turn whose steps the user expanded by hand.
pub(super) fn expansion_key(turn: usize) -> String {
    format!("steps:{turn}")
}

/// `expanded_tools` key for a turn whose steps the user folded by hand. An
/// explicit collapse wins over the in-progress turn.
fn hide_key(turn: usize) -> String {
    format!("steps-hide:{turn}")
}

/// Synthetic row index for a turn's steps-control row. It sits below the
/// applet range (`usize::MAX / 2 + ordinal`) and the live-text sentinels, so
/// it never collides with an item index or a derived row.
fn row_index(turn: usize) -> usize {
    usize::MAX / 4 + turn
}

/// Whether `turn`'s steps render folded. An explicit collapse wins over the
/// in-progress turn; an explicit expand wins over auto-fold.
pub(super) fn is_folded(
    display: &crate::config::DisplayConfig,
    turn: usize,
    expanded: &HashSet<String>,
    turn_start: Option<usize>,
) -> bool {
    if display.steps_display != crate::config::StepsDisplay::Collapsed {
        return false;
    }
    if expanded.contains(&hide_key(turn)) {
        return true;
    }
    if expanded.contains(&expansion_key(turn)) {
        return false;
    }
    turn_start != Some(turn)
}

/// Items that may close a turn as part of its visible answer: assistant
/// text, error lines, the stats footer, stop notices, and images the
/// assistant produced itself. `CacheMiss` counts too — a cache-miss
/// notice is emitted per request and can land after the final answer when
/// a follow-up request (auto-poke, compaction) starts, and it must not
/// drag the answer into the fold. A trailing `ToolResult` image stays a
/// step — it belongs to its call — and a mid-turn `Error` is a step too
/// unless it ends the turn.
fn answer_eligible(item: &Item) -> bool {
    match item {
        Item::Assistant(_)
        | Item::Error(_)
        | Item::ResponseStats(_)
        | Item::Stopped(_)
        | Item::CacheMiss(_) => true,
        Item::Image(image) => matches!(
            image.source,
            jcode_sdk::RenderedImageSource::Other { .. }
        ),
        _ => false,
    }
}

/// One turn's item ranges: the steps block is `steps_start..steps_end`.
struct Turn {
    /// `Item::User` index that opened the turn.
    user: usize,
    /// First step item: past the prompt and its `UserInput` image attachments.
    steps_start: usize,
    /// First answer item: the trailing run of `answer_eligible` items.
    steps_end: usize,
}

/// Split items into turns in one O(items) pass. Everything between a
/// `Item::User` (plus its attached `UserInput` images) and the next prompt
/// is a turn; thoughts, tool calls and results, intermediate text, notices,
/// background tasks, and mid-turn errors are steps, while the trailing
/// answer run stays visible.
fn turns(items: &Transcript) -> Vec<Turn> {
    let mut turns = Vec::new();
    let mut index = 0;
    while index < items.len() {
        if !matches!(items[index], Item::User(_)) {
            index += 1;
            continue;
        }
        let end = items[index + 1..]
            .iter()
            .position(|item| matches!(item, Item::User(_)))
            .map_or(items.len(), |offset| index + 1 + offset);
        let mut steps_start = index + 1;
        while steps_start < end
            && matches!(&items[steps_start], Item::Image(image)
                if matches!(image.source, jcode_sdk::RenderedImageSource::UserInput))
        {
            steps_start += 1;
        }
        let mut steps_end = end;
        while steps_end > steps_start && answer_eligible(&items[steps_end - 1]) {
            steps_end -= 1;
        }
        // A notice that opens the answer run belongs to the work before it.
        while steps_end < end && matches!(items[steps_end], Item::CacheMiss(_)) {
            steps_end += 1;
        }
        turns.push(Turn {
            user: index,
            steps_start,
            steps_end,
        });
        index = end;
    }
    turns
}

/// The item index a render row points at, for rows that still carry one.
/// Applet and steps-control rows use sentinel indices and return `None`, so
/// they are classified by position instead.
fn item_index(row: &TranscriptRenderRow) -> Option<usize> {
    match row.source {
        TranscriptRowSource::Settled(index) => Some(index),
        // Coalesced reasoning keeps its first item's index.
        TranscriptRowSource::Owned(_) if row.index < usize::MAX / 4 => Some(row.index),
        _ => None,
    }
}

/// Replace each foldable turn's step rows with the steps-control row — the
/// summary pill when folded, "Hide steps" above the block otherwise. Row
/// order is preserved, so a turn's steps are simply the rows between its
/// first step row and its first answer row, which also captures derived
/// applet and coalesced-reasoning rows.
pub(super) fn apply(
    items: &Transcript,
    rows: Vec<TranscriptRenderRow>,
    display: &crate::config::DisplayConfig,
    expanded: &HashSet<String>,
    turn_start: Option<usize>,
) -> Vec<TranscriptRenderRow> {
    if display.steps_display != crate::config::StepsDisplay::Collapsed {
        return rows;
    }
    let turns = turns(items);
    if turns.is_empty() {
        return rows;
    }
    let mut out: Vec<TranscriptRenderRow> = Vec::with_capacity(rows.len() + turns.len());
    let mut turns = turns.iter().peekable();
    let mut turn: Option<&Turn> = None;
    let mut folded = false;
    let mut summary = StepsFold {
        turn: 0,
        folded: false,
        tools: 0,
        failed: 0,
        thoughts: 0,
        other: 0,
        duration_secs: None,
    };
    // Step rows are buffered until the block's size is known: a one-row
    // block gains nothing from folding, so those rows pass straight back
    // out with no control row.
    let mut step_rows: Vec<TranscriptRenderRow> = Vec::new();
    // Flush the pending steps block once a turn's rows are fully read.
    macro_rules! flush {
        ($out:expr, $turn:expr, $summary:expr, $folded:expr, $step_rows:expr) => {
            if let Some(turn) = $turn.take() {
                if $step_rows.len() >= 2 {
                    $summary.folded = $folded;
                    $out.push(TranscriptRenderRow {
                        index: row_index(turn.user),
                        source: TranscriptRowSource::Owned(Box::new(Item::StepsFold(
                            $summary.clone(),
                        ))),
                        role: None,
                        show_label: false,
                    });
                    if !$folded {
                        $out.append(&mut $step_rows);
                    } else {
                        $step_rows.clear();
                    }
                } else {
                    $out.append(&mut $step_rows);
                }
            }
        };
    }
    for row in rows {
        // The end of the steps block: a settled item at or after the turn's
        // answer start, or a live streaming sentinel.
        let at_answer = item_index(&row).is_some_and(|index| {
            turn.is_some_and(|turn| index >= turn.steps_end)
        }) || row.index >= usize::MAX - 1;
        if turn.is_some() && at_answer {
            flush!(out, turn, summary, folded, step_rows);
            summary = StepsFold {
                turn: 0,
                folded: false,
                tools: 0,
                failed: 0,
                thoughts: 0,
                other: 0,
                duration_secs: None,
            };
        }
        // A settled User row opens a new turn; anything before it stays put.
        if let Some(index) = item_index(&row)
            && let Some(next) = turns.peek().copied()
            && index == next.user
        {
            turns.next();
            let duration_secs = items[next.steps_end..]
                .iter()
                .take_while(|item| !matches!(item, Item::User(_)))
                .find_map(|item| match item {
                    Item::ResponseStats(stats) => stats.duration_secs,
                    _ => None,
                });
            turn = Some(next);
            folded = is_folded(display, next.user, expanded, turn_start);
            summary = StepsFold {
                turn: next.user,
                folded,
                tools: 0,
                failed: 0,
                thoughts: 0,
                other: 0,
                duration_secs,
            };
            step_rows.clear();
            out.push(row);
            continue;
        }
        // Inside a steps block: count the row, and drop it when folded.
        if let Some(active) = turn {
            let in_steps = match item_index(&row) {
                Some(index) => index >= active.steps_start && index < active.steps_end,
                // Derived rows sit where they are emitted: inside the block
                // once it has started, outside it before the first step row.
                None => !step_rows.is_empty(),
            };
            if in_steps {
                match &row.source {
                    TranscriptRowSource::Settled(index) => match &items[*index] {
                        Item::Tool { error, .. } => {
                            summary.tools += 1;
                            summary.failed += usize::from(error.is_some());
                        }
                        Item::Reasoning(_) => summary.thoughts += 1,
                        _ => summary.other += 1,
                    },
                    TranscriptRowSource::Owned(item) => match item.as_ref() {
                        Item::Reasoning(_) => summary.thoughts += 1,
                        Item::Tool { error, .. } => {
                            summary.tools += 1;
                            summary.failed += usize::from(error.is_some());
                        }
                        _ => summary.other += 1,
                    },
                }
                step_rows.push(row);
                continue;
            }
        }
        out.push(row);
    }
    flush!(out, turn, summary, folded, step_rows);
    out
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// The pill text: `Worked for 2m 13s · 6 tool calls · 4 thoughts`, dropping
/// any part that is zero or unknown.
fn label(fold: &StepsFold) -> String {
    let chinese = is_zh();
    let mut parts = Vec::new();
    if fold.tools > 0 {
        parts.push(if chinese {
            format!("{} 次工具调用", fold.tools)
        } else {
            format!("{} tool call{}", fold.tools, plural(fold.tools))
        });
    }
    if fold.thoughts > 0 {
        parts.push(if chinese {
            format!("{} 段思考", fold.thoughts)
        } else {
            format!("{} thought{}", fold.thoughts, plural(fold.thoughts))
        });
    }
    if fold.other > 0 {
        parts.push(if chinese {
            format!("{} 个步骤", fold.other)
        } else {
            format!("{} step{}", fold.other, plural(fold.other))
        });
    }
    if fold.failed > 0 {
        parts.push(if chinese {
            format!("{} 项失败", fold.failed)
        } else {
            format!("{} failed", fold.failed)
        });
    }
    let mut label = String::from(tr("Worked", "已执行"));
    if let Some(secs) = fold.duration_secs {
        label = format!(
            "{} {}",
            tr("Worked for", "耗时"),
            format_tool_elapsed(Duration::from_secs_f64(secs.max(0.)))
        );
    }
    if parts.is_empty() {
        label
    } else {
        format!("{label} · {}", parts.join(" · "))
    }
}

impl Panel {
    /// Keep this turn's steps expanded until folded again.
    fn expand_steps(&mut self, turn: usize, cx: &mut Context<Self>) {
        self.expanded_tools.remove(&hide_key(turn));
        self.expanded_tools.insert(expansion_key(turn));
        self.transcript_measurements.dirty = true;
        cx.notify();
    }

    /// Fold this turn's steps, even while the turn still runs.
    fn collapse_steps(&mut self, turn: usize, cx: &mut Context<Self>) {
        self.expanded_tools.remove(&expansion_key(turn));
        self.expanded_tools.insert(hide_key(turn));
        self.transcript_measurements.dirty = true;
        cx.notify();
    }

    /// `alt-t` / `/steps toggle`: expand every foldable turn's steps, or
    /// fold them all back when none is folded. Returns true when it expanded.
    pub(super) fn toggle_all_steps(&mut self, cx: &mut Context<Self>) -> bool {
        let display = crate::config::display();
        let turn_start = self.current_turn_start();
        let turn_indices: Vec<usize> = turns(&self.items)
            .into_iter()
            .filter(|turn| turn.steps_end.saturating_sub(turn.steps_start) >= 2)
            .map(|turn| turn.user)
            .collect();
        let expand = turn_indices
            .iter()
            .any(|&turn| is_folded(&display, turn, &self.expanded_tools, turn_start));
        for turn in turn_indices {
            if expand {
                self.expanded_tools.remove(&hide_key(turn));
                self.expanded_tools.insert(expansion_key(turn));
            } else {
                self.expanded_tools.remove(&expansion_key(turn));
                self.expanded_tools.insert(hide_key(turn));
            }
        }
        self.transcript_measurements.dirty = true;
        cx.notify();
        expand
    }

    /// The folded summary pill (click expands the steps in place) or, when
    /// the block is expanded, the compact "Hide steps" control above it.
    pub(super) fn render_steps_fold(
        &self,
        fold: &StepsFold,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let turn = fold.turn;
        if !fold.folded {
            return div()
                .id(("steps-unfold", turn))
                .debug_selector(move || format!("steps-unfold-{turn}").into())
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
                    panel.collapse_steps(turn, cx);
                }))
                .child(tr("Hide steps", "收起步骤"))
                .into_any_element();
        }
        div()
            .id(("steps-fold", turn))
            .debug_selector(move || format!("steps-fold-{turn}").into())
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
            .text_color(theme.TEXT_DIM)
            .cursor_pointer()
            .hover(|style| style.text_color(theme.TEXT))
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.expand_steps(turn, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .child(label(fold)),
            )
            .child(div().flex_1().min_w_0())
            .child(
                div()
                    .flex_none()
                    .text_color(theme.TEXT_FAINT)
                    .child(tr("Show steps", "展开步骤")),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collapsed() -> crate::config::DisplayConfig {
        crate::config::DisplayConfig {
            steps_display: crate::config::StepsDisplay::Collapsed,
            ..Default::default()
        }
    }

    fn user() -> Item {
        Item::User("prompt".into())
    }

    fn tool(done_error: bool) -> Item {
        Item::Tool {
            call_id: "c".into(),
            name: "bash".into(),
            input: "{}".into(),
            output: "out".into(),
            done: true,
            error: done_error.then(|| "boom".into()),
        }
    }

    #[test]
    fn a_turn_splits_into_prompt_steps_and_answer() {
        let items: Transcript = vec![
            user(),
            Item::Reasoning("think".into()),
            tool(false),
            Item::Assistant("intermediate".into()),
            tool(false),
            Item::Assistant("final".into()),
            Item::ResponseStats(response_stats::fixture()),
        ]
        .into();
        let turns = turns(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!((turns[0].steps_start, turns[0].steps_end), (1, 5));
    }

    /// A cache-miss notice after the answer rides with the visible tail
    /// instead of dragging the answer into the folded steps.
    #[test]
    fn a_trailing_cache_miss_stays_with_the_answer() {
        let items: Transcript = vec![
            user(),
            tool(false),
            Item::Assistant("final".into()),
            Item::ResponseStats(response_stats::fixture()),
            Item::CacheMiss(cache_miss::CacheMissNotice {
                reason: "provider restart".into(),
                harness_caused: false,
                documented_cause: None,
                missed_tokens: 35_000,
                message: "KV cache miss".into(),
            }),
        ]
        .into();
        let turns = turns(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!((turns[0].steps_start, turns[0].steps_end), (1, 2));
    }

    /// A cache-miss notice just before the answer stays folded with the work.
    #[test]
    fn a_cache_miss_before_the_answer_folds_with_the_steps() {
        let items: Transcript = vec![
            user(),
            tool(false),
            Item::CacheMiss(cache_miss::CacheMissNotice {
                reason: "provider restart".into(),
                harness_caused: false,
                documented_cause: None,
                missed_tokens: 35_000,
                message: "KV cache miss".into(),
            }),
            Item::Assistant("final".into()),
            Item::ResponseStats(response_stats::fixture()),
        ]
        .into();
        let turns = turns(&items);
        assert_eq!((turns[0].steps_start, turns[0].steps_end), (1, 3));
    }

    #[test]
    fn folds_only_with_two_or_more_steps_and_respects_state() {
        let mut expanded = HashSet::new();
        assert!(is_folded(&collapsed(), 0, &expanded, None));
        assert!(!is_folded(&collapsed(), 0, &expanded, Some(0)));
        expanded.insert(expansion_key(0));
        assert!(!is_folded(&collapsed(), 0, &expanded, None));
        let mut expanded = HashSet::new();
        expanded.insert(hide_key(0));
        assert!(is_folded(&collapsed(), 0, &expanded, Some(0)));
        let full = crate::config::DisplayConfig {
            steps_display: crate::config::StepsDisplay::Expanded,
            ..Default::default()
        };
        assert!(!is_folded(&full, 0, &HashSet::new(), None));
    }

    #[test]
    fn the_fold_label_omits_empty_parts() {
        let fold = StepsFold {
            turn: 0,
            folded: true,
            tools: 6,
            failed: 2,
            thoughts: 1,
            other: 0,
            duration_secs: Some(133.),
        };
        assert_eq!(
            label(&fold),
            "Worked for 2m 13s · 6 tool calls · 1 thought · 2 failed"
        );
        let quiet = StepsFold {
            duration_secs: None,
            tools: 0,
            failed: 0,
            thoughts: 0,
            other: 3,
            ..fold
        };
        assert_eq!(label(&quiet), "Worked · 3 steps");
    }

    #[test]
    fn fold_label_localizes_counts_and_elapsed_time() {
        let fold = StepsFold {
            turn: 0,
            folded: true,
            tools: 6,
            failed: 2,
            thoughts: 1,
            other: 3,
            duration_secs: Some(133.),
        };
        let untimed = StepsFold { duration_secs: None, ..fold };
        assert_eq!(
            label(&fold),
            "Worked for 2m 13s · 6 tool calls · 1 thought · 3 steps · 2 failed"
        );
        assert_eq!(label(&untimed), "Worked · 6 tool calls · 1 thought · 3 steps · 2 failed");
        crate::locale::with_zh(|| {
            assert_eq!(label(&fold), "耗时 2m 13s · 6 次工具调用 · 1 段思考 · 3 个步骤 · 2 项失败");
            assert_eq!(label(&untimed), "已执行 · 6 次工具调用 · 1 段思考 · 3 个步骤 · 2 项失败");
        });
    }

    fn steps_fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<Panel>, &mut gpui::VisualTestContext) {
        crate::config::set_display("steps_display", "collapsed").unwrap();
        cx.add_window_view(|_, cx| {
            Panel::new(
                "steps-test".into(),
                None,
                None,
                crate::harness::spawn_inert(),
                cx,
            )
        })
    }

    fn session_status(status: &str) -> ApiEvent {
        ApiEvent::SessionStatus {
            session_id: "steps-test".into(),
            status: status.into(),
            pending_soft_interrupts: None,
        }
    }

    fn work_items() -> Vec<Item> {
        vec![
            Item::User("prompt".into()),
            Item::Reasoning("thinking".into()),
            tool(false),
            tool(true),
            tool(false),
            Item::CacheMiss(cache_miss::CacheMissNotice {
                reason: "provider restart".into(),
                harness_caused: false,
                documented_cause: None,
                missed_tokens: 35_000,
                message: "KV cache miss".into(),
            }),
            Item::Assistant("Let me try that.".into()),
            tool(false),
            Item::Assistant("The final answer.".into()),
            Item::ResponseStats(response_stats::fixture()),
        ]
    }

    /// The reported real transcript: prompt, thoughts, tool batches and
    /// cache-miss notices, then the long final answer and its stats row.
    fn reattach_turn_items() -> Vec<Item> {
        let miss = || {
            Item::CacheMiss(cache_miss::CacheMissNotice {
                reason: "provider restart".into(),
                harness_caused: false,
                documented_cause: None,
                missed_tokens: 35_000,
                message: "KV cache miss".into(),
            })
        };
        vec![
            Item::User("整理一下最新的 AI 模型动态".into()),
            Item::Reasoning("thinking".into()),
            Item::Assistant("我来搜索最新的 AI 模型动态。".into()),
            tool(false),
            Item::Reasoning("thinking".into()),
            tool(false),
            miss(),
            Item::Reasoning("thinking".into()),
            Item::Assistant("再确认一下细节。".into()),
            tool(false),
            miss(),
            Item::Reasoning("thinking".into()),
            Item::Assistant("The final answer with **markdown**.".into()),
            Item::ResponseStats(response_stats::fixture()),
        ]
    }

    /// Reattached or reconnecting panels hold non-`idle` statuses
    /// (`lost: …`, `connected`, `crashed`) that are not real activity. A
    /// finished turn must still fold under them — the bug this guards.
    #[gpui::test]
    fn a_settled_turn_folds_under_transport_and_lost_statuses(
        cx: &mut gpui::TestAppContext,
    ) {
        for status in [
            "lost: harness connection closed; reconnecting",
            "connected",
            "crashed",
        ] {
            let (panel, vcx) = steps_fixture(cx);
            panel.update(vcx, |panel, cx| {
                *panel.items = reattach_turn_items();
                panel.status = status.into();
                cx.notify();
            });
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("steps-fold-0").is_some(),
                "{status}: the finished turn folds"
            );
            assert!(
                vcx.debug_bounds("assistant-response").is_some(),
                "{status}: the final answer stays visible"
            );
            assert!(
                vcx.debug_bounds("tool-inline").is_none(),
                "{status}: tool rows fold away"
            );
            assert!(
                vcx.debug_bounds("reasoning-fold-1").is_none(),
                "{status}: thought rows fold away"
            );
            let rows = panel.update(vcx, |panel, _cx| panel.test_transcript_row_count());
            assert_eq!(rows, 4, "{status}: prompt + fold pill + answer + stats");
        }
    }

    /// The same turn stays expanded while it is genuinely running.
    #[gpui::test]
    fn the_live_turn_stays_expanded_while_running(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = reattach_turn_items();
            panel.status = "running".into();
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_none());
        assert!(vcx.debug_bounds("steps-unfold-0").is_some());
        assert!(vcx.debug_bounds("tool-inline").is_some());
    }

    #[gpui::test]
    fn a_finished_turn_folds_its_steps(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = work_items();
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_some());
        assert!(
            vcx.debug_bounds("assistant-response").is_some(),
            "the final answer stays visible"
        );
        assert!(
            vcx.debug_bounds("tool-inline").is_none(),
            "tool rows fold away"
        );
        let rows = panel.update(vcx, |panel, _cx| panel.test_transcript_row_count());
        assert_eq!(rows, 4, "prompt + fold pill + answer + stats");
    }

    #[gpui::test]
    fn the_running_turn_keeps_steps_and_folds_when_it_finishes(
        cx: &mut gpui::TestAppContext,
    ) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = work_items();
            panel.apply(&session_status("processing"), cx);
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("steps-fold-0").is_none(),
            "the running turn's steps stay expanded"
        );
        assert!(vcx.debug_bounds("steps-unfold-0").is_some());
        assert!(vcx.debug_bounds("tool-inline").is_some());
        panel.update(vcx, |panel, cx| panel.apply(&session_status("idle"), cx));
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("steps-fold-0").is_some(),
            "steps fold without a click once the turn finishes"
        );
        assert!(vcx.debug_bounds("tool-inline").is_none());
    }

    #[gpui::test]
    fn pill_and_hide_steps_roundtrip(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = work_items();
            cx.notify();
        });
        vcx.run_until_parked();
        let pill = vcx.debug_bounds("steps-fold-0").unwrap();
        vcx.simulate_click(pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_none());
        assert!(vcx.debug_bounds("tool-inline").is_some());
        let hide = vcx.debug_bounds("steps-unfold-0").unwrap();
        vcx.simulate_click(hide.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_some());
        // An explicit collapse persists into the next turn.
        panel.update(vcx, |panel, cx| {
            panel.items.push(Item::User("next".into()));
            panel.items.push(Item::Reasoning("more".into()));
            panel.items.push(tool(false));
            panel.items.push(Item::Assistant("working".into()));
            panel.apply(&session_status("processing"), cx);
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("steps-fold-0").is_some(),
            "the explicit collapse survives a new turn"
        );
        // And the new running turn's steps stay expanded.
        assert!(vcx.debug_bounds("steps-unfold-10").is_some());
    }

    #[gpui::test]
    fn an_errored_turn_folds_steps_but_keeps_the_error(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = vec![
                Item::User("prompt".into()),
                Item::Reasoning("thinking".into()),
                tool(true),
                Item::Error("the request failed".into()),
            ];
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_some());
        assert!(vcx.debug_bounds("tool-inline").is_none());
        let rows = panel.update(vcx, |panel, _cx| panel.test_transcript_row_count());
        assert_eq!(rows, 3, "prompt + fold pill + error");
    }

    #[gpui::test]
    fn a_single_step_turn_stays_expanded(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            *panel.items = vec![
                Item::User("prompt".into()),
                tool(false),
                Item::Assistant("answer".into()),
            ];
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_none());
        assert!(vcx.debug_bounds("steps-unfold-0").is_none());
        assert!(vcx.debug_bounds("tool-inline").is_some());
    }

    #[gpui::test]
    fn alt_t_and_slash_steps_toggle_every_turn(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        panel.update(vcx, |panel, cx| {
            let mut items = work_items();
            items.push(Item::User("second".into()));
            items.push(Item::Reasoning("again".into()));
            items.push(tool(false));
            items.push(tool(false));
            items.push(Item::Assistant("second answer".into()));
            *panel.items = items;
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_some());
        assert!(vcx.debug_bounds("steps-fold-10").is_some());
        let dispatch = |vcx: &mut gpui::VisualTestContext| {
            vcx.update(|window, cx| {
                let focus = panel.read(cx).input.read(cx).focus_handle.clone();
                focus.dispatch_action(&reasoning_fold::ToggleAllThinking, window, cx);
            });
            vcx.run_until_parked();
        };
        dispatch(vcx);
        assert!(
            vcx.debug_bounds("steps-fold-0").is_none()
                && vcx.debug_bounds("steps-fold-10").is_none(),
            "alt-t expands every turn"
        );
        dispatch(vcx);
        assert!(
            vcx.debug_bounds("steps-fold-0").is_some()
                && vcx.debug_bounds("steps-fold-10").is_some(),
            "alt-t folds them all back"
        );
        // `/steps expanded` disables folding entirely.
        panel.update(vcx, |panel, cx| {
            assert!(panel.handle_slash_command_for_test("/steps expanded", cx));
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("steps-fold-0").is_none()
                && vcx.debug_bounds("steps-fold-10").is_none()
        );
        crate::config::set_display("steps_display", "collapsed").unwrap();
    }

    #[gpui::test]
    fn reply_nav_lands_on_a_folded_turns_prompt(cx: &mut gpui::TestAppContext) {
        let (panel, vcx) = steps_fixture(cx);
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(700.), px(600.)));
        panel.update(vcx, |panel, cx| {
            let mut items = work_items();
            items.push(Item::User("second".into()));
            for index in 0..20 {
                items.push(Item::Assistant(format!("filler {index}")));
            }
            items.push(Item::Reasoning("again".into()));
            items.push(tool(false));
            items.push(tool(false));
            items.push(Item::Assistant("second answer".into()));
            *panel.items = items;
            panel.stick_to_bottom = true;
            cx.notify();
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("steps-fold-0").is_some());
        let prompt_rows: Vec<usize> = panel.update(vcx, |panel, cx| {
            panel
                .transcript_derived(cx)
                .prompt_rows
                .iter()
                .map(|(row, _)| *row)
                .collect()
        });
        assert_eq!(prompt_rows.len(), 2);
        panel.update(vcx, |panel, cx| panel.step_prompt(-1, cx));
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert_eq!(
                panel.transcript_list.logical_scroll_top().item_ix,
                prompt_rows[0],
                "the jump lands on the folded turn's prompt row"
            );
        });
    }

    #[test]
    fn folded_steps_leave_the_selection_document() {
        let items: Transcript = vec![
            Item::User("prompt".into()),
            Item::Tool {
                call_id: "a".into(),
                name: "bash".into(),
                input: "{}".into(),
                output: "SECRET-OUTPUT".into(),
                done: true,
                error: None,
            },
            Item::Tool {
                call_id: "b".into(),
                name: "bash".into(),
                input: "{}".into(),
                output: "ALSO-SECRET".into(),
                done: true,
                error: None,
            },
            Item::Assistant("the visible answer".into()),
        ]
        .into();
        let base_rows: Vec<TranscriptRenderRow> = items
            .iter()
            .enumerate()
            .map(|(index, item)| TranscriptRenderRow {
                index,
                source: TranscriptRowSource::Settled(index),
                role: role_of(item),
                show_label: false,
            })
            .collect();
        let folded = apply(&items, base_rows, &collapsed(), &HashSet::new(), None);
        let mut document = super::super::text_document::TranscriptTextDocument::default();
        let segments = document
            .sync(&items, &folded, &HashSet::new(), &HashSet::new(), None)
            .unwrap_or_default();
        let text: String = segments
            .iter()
            .map(|(id, text)| format!("{id} {text}"))
            .collect();
        assert!(!text.contains("SECRET"));
        assert!(text.contains("the visible answer"));
    }
}
