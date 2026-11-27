//! Numbered sign-in checklist. Each step reflects confirmed runtime state only:
//! a step is marked done when the flow proves it, the current step is
//! expanded, and finished or upcoming steps collapse to one pill row.
use super::*;
use gpui::{Animation, AnimationExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepKind {
    Provider,
    Prepare,
    Approve,
    Connect,
    EnterKey,
    SaveKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepStatus {
    Done,
    Active,
    Failed,
    Pending,
}

/// The facts that drive the checklist, separated from GPUI for unit tests.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Progress {
    pub api_key: bool,
    pub prompt: bool,
    pub busy: bool,
    pub authorized: bool,
    pub complete: bool,
    pub error: bool,
}

impl Progress {
    pub(super) fn of(state: &LoginState) -> Self {
        Self {
            api_key: state
                .provider
                .is_some_and(|provider| provider.method == LoginMethod::ApiKey),
            prompt: state.prompt.is_some(),
            busy: state.busy,
            authorized: state.authorized,
            complete: state.complete,
            error: state.error.is_some(),
        }
    }
}

pub(super) fn steps(progress: Progress) -> Vec<(StepKind, StepStatus)> {
    let (kinds, current): (&[StepKind], usize) = if progress.api_key {
        let current = if progress.complete {
            3
        } else if progress.busy {
            2
        } else {
            1
        };
        (
            &[StepKind::Provider, StepKind::EnterKey, StepKind::SaveKey],
            current,
        )
    } else {
        // Completion clears the prompt, so check it first.
        let current = if progress.complete {
            4
        } else if progress.authorized {
            3
        } else if progress.prompt {
            2
        } else {
            1
        };
        (
            &[
                StepKind::Provider,
                StepKind::Prepare,
                StepKind::Approve,
                StepKind::Connect,
            ],
            current,
        )
    };
    kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            let status = match index.cmp(&current) {
                std::cmp::Ordering::Less => StepStatus::Done,
                std::cmp::Ordering::Equal if progress.error => StepStatus::Failed,
                std::cmp::Ordering::Equal => StepStatus::Active,
                std::cmp::Ordering::Greater => StepStatus::Pending,
            };
            (*kind, status)
        })
        .collect()
}

fn title(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Provider => tr("Choose provider", "选择服务商"),
        StepKind::Prepare => tr("Prepare secure sign-in", "准备安全登录"),
        StepKind::Approve => tr("Approve access", "批准访问"),
        StepKind::Connect => tr("Connect account", "连接账户"),
        StepKind::EnterKey => tr("Paste API key", "粘贴 API 密钥"),
        StepKind::SaveKey => tr("Save key securely", "安全保存密钥"),
    }
}

/// Short confirmation shown on a collapsed, finished step.
fn done_summary(kind: StepKind, state: &LoginState) -> &'static str {
    match kind {
        StepKind::Provider => state
            .provider
            .map(|provider| provider.display_name)
            .unwrap_or(tr("Selected", "已选择")),
        StepKind::Prepare => tr("Sign-in link ready", "登录链接已就绪"),
        StepKind::Approve if state.authorized => tr("Approved in browser", "已在浏览器中授权"),
        StepKind::Approve => tr("Approved", "已授权"),
        StepKind::Connect => tr("Connected", "已连接"),
        StepKind::EnterKey => tr("Key received", "已接收密钥"),
        StepKind::SaveKey => tr("Saved", "已保存"),
    }
}

/// Indeterminate progress: a pill track with a segment sliding across it.
/// GPUI's animation wrapper honors reduced motion.
fn loading_bar() -> gpui::AnyElement {
    let theme = Theme::global();
    div()
        .debug_selector(|| "login-loading-bar".into())
        .relative()
        .w_full()
        .max_w(px(320.))
        .h(px(4.))
        .rounded_full()
        .overflow_hidden()
        .bg(theme.ACCENT_DIM)
        .child(
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(relative(0.35))
                .rounded_full()
                .bg(theme.ACCENT)
                .with_animation(
                    "login-loading-slide",
                    Animation::new(std::time::Duration::from_millis(1300))
                        .repeat()
                        .with_max_fps(60.0)
                        .with_easing(gpui::ease_in_out),
                    |segment, phase| segment.left(relative(-0.35 + 1.35 * phase)),
                ),
        )
        .into_any_element()
}

/// Only reveal the manual code field when it is actually the way forward:
/// no automatic browser return exists, or the user asked for it.
pub(super) fn manual_input_visible(state: &LoginState) -> bool {
    !state.complete
        && !state.busy
        && !state.authorized
        && state
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.input_kind != AuthInputKind::DeviceCode)
        && (state.manual_entry || !state.callback_waiting)
}

impl Panel {
    pub(super) fn render_login_steps(
        &self,
        state: &LoginState,
        provider: LoginProvider,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let mut list = div()
            .debug_selector(|| "login-steps".into())
            .flex()
            .flex_col()
            .gap_2();
        for (index, (kind, status)) in steps(Progress::of(state)).into_iter().enumerate() {
            let number = index + 1;
            let expanded = matches!(status, StepStatus::Active | StepStatus::Failed);
            let badge = div()
                .size(px(22.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .map(|el| match status {
                    StepStatus::Done => el.bg(theme.ACCENT_DIM).text_color(theme.OK).child("✓"),
                    StepStatus::Active => el
                        .bg(theme.ACCENT)
                        .text_color(theme.PANEL_BG)
                        .child(number.to_string()),
                    StepStatus::Failed => el
                        .bg(theme.ERROR_BG)
                        .text_color(theme.ERROR)
                        .child(number.to_string()),
                    StepStatus::Pending => el
                        .bg(theme.ACCENT_DIM)
                        .text_color(theme.TEXT_FAINT)
                        .child(number.to_string()),
                });
            let header = div()
                .flex()
                .items_center()
                .gap_3()
                .child(badge)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(if expanded {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(match status {
                            StepStatus::Pending => theme.TEXT_FAINT,
                            StepStatus::Done => theme.TEXT_DIM,
                            _ => theme.TEXT,
                        })
                        .child(title(kind)),
                )
                .when(status == StepStatus::Done, |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .child(done_summary(kind, state)),
                    )
                })
                // Changing provider is always one step back, never a dead end.
                .when(
                    kind == StepKind::Provider && !state.complete && !state.busy,
                    |el| {
                        el.child(
                            login_button("login-back", tr("Change", "更换"))
                                .px_3()
                                .py_0p5()
                                .text_size(px(12.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_login_picker(cx);
                                    this.open_login_picker(cx);
                                })),
                        )
                    },
                );
            let step_id = format!("login-step-{number}");
            let row = div().debug_selector(move || step_id.clone());
            list = list.child(if expanded {
                row.child(
                    div()
                        .debug_selector(|| "login-current-step".into())
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_3()
                        .rounded_xl()
                        .bg(theme.HEADER_BG)
                        .child(header)
                        .child(
                            div()
                                .pl(px(34.))
                                .flex()
                                .flex_col()
                                .gap_3()
                                .children(self.render_step_body(kind, state, provider, cx)),
                        ),
                )
            } else {
                row.px_3()
                    .py_1p5()
                    .rounded_full()
                    .when(status == StepStatus::Done, |el| el.bg(theme.HEADER_BG))
                    .child(header)
            });
        }
        list.into_any_element()
    }

    fn render_step_body(
        &self,
        kind: StepKind,
        state: &LoginState,
        provider: LoginProvider,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let theme = Theme::global();
        let mut body: Vec<gpui::AnyElement> = Vec::new();
        let status_line = |text: &'static str, busy: bool| {
            let line = div().text_color(theme.TEXT_DIM).child(text);
            if busy {
                div()
                    .debug_selector(|| "login-busy".into())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(line)
                    .child(loading_bar())
                    .into_any_element()
            } else {
                line.into_any_element()
            }
        };
        if let Some(error) = &state.error {
            body.push(
                div()
                    .debug_selector(|| "login-error".into())
                    .text_color(theme.ERROR)
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        match kind {
            StepKind::Provider => {}
            StepKind::Prepare if state.error.is_none() => {
                body.push(status_line(tr("Creating a private sign-in link…", "正在创建私密登录链接…"), state.busy));
            }
            StepKind::Prepare => {}
            StepKind::Approve => {
                let Some(prompt) = &state.prompt else {
                    return body;
                };
                let device = prompt.input_kind == AuthInputKind::DeviceCode;
                if state.error.is_none() {
                    body.push(status_line(
                        if state.busy && !device {
                            tr("Checking your code…", "正在验证授权码…")
                        } else if device {
                            tr("Enter the code below on the sign-in page. This window continues automatically.", "请在登录页面输入下方设备码。此窗口会自动继续。")
                        } else if state.callback_waiting {
                            tr("We opened the sign-in page in your browser. Approve access there and this step completes automatically.", "登录页面已在浏览器中打开。授权后，此步骤会自动完成。")
                        } else {
                            tr("Approve access in your browser, then paste the code it shows.", "请在浏览器中授权，然后粘贴页面显示的授权码。")
                        },
                        state.busy,
                    ));
                }
                if !state.busy || device {
                    let url = prompt.auth_url.clone();
                    let copy_url = url.clone();
                    let preview = self.preview_state.is_some() || crate::harness::screenshot_mode();
                    let mut actions = div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            login_button("login-open-browser", tr("Open sign-in page again", "重新打开登录页面")).on_click(
                                move |_, _, cx| {
                                    if !preview {
                                        cx.open_url(&url);
                                    }
                                },
                            ),
                        )
                        .child(login_button("login-copy-link", tr("Copy link", "复制链接")).on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy_url.clone(),
                                ));
                            },
                        ));
                    if state.callback_waiting && !state.manual_entry {
                        actions = actions.child(
                            login_button("login-manual-entry", tr("Paste callback URL instead", "改为粘贴回调 URL"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.login.as_mut() {
                                        state.manual_entry = true;
                                        state.focus_pending = true;
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    body.push(actions.into_any_element());
                }
                if let Some(code) = &prompt.user_code {
                    let code = code.clone();
                    body.push(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(theme.FONT_MONO)
                                    .child(format!("{}{code}", tr("Device code: ", "设备码："))),
                            )
                            .child(login_button("login-copy-code", tr("Copy code", "复制设备码")).on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        code.clone(),
                                    ));
                                },
                            ))
                            .into_any_element(),
                    );
                }
                if manual_input_visible(state) {
                    body.push(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .child(if state.callback_waiting {
                                tr("Paste the full address your browser ended on. It stays private.", "请粘贴浏览器最终跳转到的完整地址。该地址会保密。")
                            } else if prompt.input_kind == AuthInputKind::CallbackUrl {
                                tr("Automatic return is unavailable here. After approving, paste the full localhost address from your browser. It stays private.", "此处无法自动返回。授权后，请粘贴浏览器中完整的 localhost 地址。该地址会保密。")
                            } else {
                                tr("The code stays private and is never sent to chat.", "授权码会保密，不会发送到聊天中。")
                            })
                            .into_any_element(),
                    );
                    body.push(state.input.clone().into_any_element());
                    body.push(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                login_button("login-paste", tr("Paste from clipboard", "从剪贴板粘贴")).on_click(
                                    cx.listener(|this, _, window, cx| this.paste_login(window, cx)),
                                ),
                            )
                            .child(
                                login_button("login-submit", tr("Finish sign-in", "完成登录"))
                                    .on_click(cx.listener(|this, _, _, cx| this.submit_login(cx))),
                            )
                            .into_any_element(),
                    );
                }
            }
            StepKind::Connect => {
                body.push(status_line(
                    tr("Saving your credentials and testing the connection…", "正在保存凭据并测试连接…"),
                    state.busy,
                ));
            }
            StepKind::EnterKey => {
                body.push(
                    div()
                        .text_color(theme.TEXT_DIM)
                        .child(
                            tr("Your key is saved in Jcode's private credential store, never in chat.", "密钥保存在 Jcode 的私有凭据库中，不会发送到聊天中。"),
                        )
                        .into_any_element(),
                );
                body.push(state.input.clone().into_any_element());
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            login_button("login-paste", tr("Paste from clipboard", "从剪贴板粘贴")).on_click(
                                cx.listener(|this, _, window, cx| this.paste_login(window, cx)),
                            ),
                        )
                        .child(
                            login_button("login-submit", tr("Connect account", "连接账户"))
                                .on_click(cx.listener(|this, _, _, cx| this.submit_login(cx))),
                        )
                        .into_any_element(),
                );
            }
            StepKind::SaveKey => {
                body.push(status_line(tr("Saving your API key securely…", "正在安全保存 API 密钥…"), state.busy));
            }
        }
        // A failed attempt can always restart from a fresh link, except when
        // the only problem was an empty field the user can fix in place.
        if state.error.is_some() && !(kind == StepKind::EnterKey && !state.busy) {
            body.push(
                login_button("login-retry", tr("Start a new sign-in", "重新开始登录"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_login_provider(provider, cx)),
                    )
                    .into_any_element(),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statuses(progress: Progress) -> Vec<StepStatus> {
        steps(progress)
            .into_iter()
            .map(|(_, status)| status)
            .collect()
    }

    use StepStatus::{Active, Done, Failed, Pending};

    #[gpui::test]
    fn chinese_sign_in_checklist_localizes_titles_and_confirmations(cx: &mut gpui::TestAppContext) {
        crate::locale::with_zh(|| {
            let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
            panel.update(vcx, |panel, cx| {
                panel.open_login_picker(cx);
                let state = panel.login.as_mut().unwrap();
                for (kind, heading, summary) in [
                    (StepKind::Provider, "选择服务商", "已选择"),
                    (StepKind::Prepare, "准备安全登录", "登录链接已就绪"),
                    (StepKind::Approve, "批准访问", "已授权"),
                    (StepKind::Connect, "连接账户", "已连接"),
                    (StepKind::EnterKey, "粘贴 API 密钥", "已接收密钥"),
                    (StepKind::SaveKey, "安全保存密钥", "已保存"),
                ] {
                    assert_eq!(title(kind), heading);
                    assert_eq!(done_summary(kind, state), summary);
                }
                state.provider = state.providers.iter().copied().find(|p| p.method == LoginMethod::ApiKey);
                assert!(state.provider.is_some());
                cx.notify();
            });
            vcx.run_until_parked();
            assert!(vcx.debug_bounds("login-current-step").is_some());
            assert!(vcx.debug_bounds("login-paste").is_some());
            assert!(vcx.debug_bounds("login-submit").is_some());
        });
    }

    #[test]
    fn oauth_steps_advance_only_on_confirmed_state() {
        let mut progress = Progress::default();
        assert_eq!(statuses(progress), [Done, Active, Pending, Pending]);
        progress.prompt = true;
        assert_eq!(statuses(progress), [Done, Done, Active, Pending]);
        // A manual code submission is still being checked, not yet approved.
        progress.busy = true;
        assert_eq!(statuses(progress), [Done, Done, Active, Pending]);
        progress.authorized = true;
        assert_eq!(statuses(progress), [Done, Done, Done, Active]);
        progress.complete = true;
        progress.prompt = false;
        assert_eq!(statuses(progress), [Done, Done, Done, Done]);
    }

    #[test]
    fn errors_fail_the_current_step_but_not_a_saved_login() {
        let failed_prepare = Progress {
            error: true,
            ..Default::default()
        };
        assert_eq!(statuses(failed_prepare), [Done, Failed, Pending, Pending]);
        let warned = Progress {
            error: true,
            complete: true,
            ..Default::default()
        };
        assert_eq!(statuses(warned), [Done, Done, Done, Done]);
    }

    #[test]
    fn api_key_steps_are_shorter() {
        let mut progress = Progress {
            api_key: true,
            ..Default::default()
        };
        assert_eq!(statuses(progress), [Done, Active, Pending]);
        progress.busy = true;
        assert_eq!(statuses(progress), [Done, Done, Active]);
        progress.complete = true;
        assert_eq!(statuses(progress), [Done, Done, Done]);
    }
}
