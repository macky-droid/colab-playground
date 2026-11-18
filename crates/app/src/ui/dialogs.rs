//! The account dialogs: add (sign in with a pasted code), switch (with what it affects) and
//! remove. Drawn by the root view so they open from any page or the sidebar switcher. Every
//! button carries a selector (`dialog-cancel`, `add-continue`, `add-submit`,
//! `add-start-again`, `add-open-again`, `switch-confirm`, `remove-confirm`).

use std::{rc::Rc, time::SystemTime};

use colab_playground_core::backend::SwitchImpact;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant, CopyButton},
    feedback::InlineMessage,
    forms::Input,
    motion::Spinner,
    overlays::Dialog,
    primitives::{IconName, Severity},
    theme::{ActiveTheme, IconSize, TextSize},
    typography::{Caption, MiddleEllipsis, Paragraph},
};
use gpui::{
    AnyElement, App, Entity, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window,
    div,
};

use crate::{
    format,
    i18n::{keys, tr, tr_args},
    state::{AccountDialog, AddPhase, AppState, Load, Op},
    ui::{Inputs, error_view, tagged},
};

pub fn render(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> Option<AnyElement> {
    let dialog = state.dialog()?;
    Some(match dialog {
        AccountDialog::Add(phase) => add(phase, state, handle, inputs, cx),
        AccountDialog::Switch {
            email,
            impact,
            error,
            ..
        } => switch(email.as_deref(), impact, error.as_ref(), state, handle, cx),
        AccountDialog::Remove { email, error, .. } => {
            remove(email.as_deref(), error.as_ref(), state, handle, cx)
        }
    })
}

fn close(handle: &Entity<AppState>) -> impl Fn(&mut Window, &mut App) + 'static {
    let handle = handle.clone();
    move |_, cx| handle.update(cx, |state, cx| state.close_dialog(cx))
}

fn cancel_button(label: SharedString, handle: Entity<AppState>) -> impl IntoElement {
    tagged(
        "dialog-cancel",
        Button::new("dialog-cancel", label)
            .variant(ButtonVariant::Ghost)
            .on_click(move |_, _, cx| handle.update(cx, |state, cx| state.close_dialog(cx))),
    )
}

fn busy_line(id: &'static str, text: SharedString) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(Spinner::new(id).size(IconSize::Sm))
        .child(Caption::new(text))
}

fn add(
    phase: &AddPhase,
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> AnyElement {
    let dialog = Dialog::new("add-account", tr(cx, keys::ADD_TITLE), close(handle));
    let cancel_label = tr(cx, keys::ACTION_CANCEL);
    let continue_label = tr(cx, keys::ADD_CONTINUE);
    let submit_label = tr(cx, keys::ADD_SUBMIT);
    let again_label = tr(cx, keys::ADD_START_AGAIN);
    let dialog = match phase {
        AddPhase::Intro => {
            let start = handle.clone();
            dialog
                .child(Paragraph::new(tr(cx, keys::ADD_INTRO)))
                .child(InlineMessage::new(Severity::Info, tr(cx, keys::ADD_IMPACT)))
                .action({
                    let handle = handle.clone();
                    move |_| cancel_button(cancel_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "add-continue",
                        Button::new("add-continue", continue_label)
                            .primary()
                            .icon(IconName::ExternalLink)
                            .on_click(move |_, _, cx| {
                                start.update(cx, |state, cx| state.begin_sign_in(cx))
                            }),
                    )
                })
        }
        AddPhase::Starting => dialog
            .child(busy_line("add-starting", tr(cx, keys::ADD_STARTING)))
            .action({
                let handle = handle.clone();
                move |_| cancel_button(cancel_label, handle)
            }),
        AddPhase::Code { url } | AddPhase::Submitting { url } => {
            let submitting = matches!(phase, AddPhase::Submitting { .. });
            let theme = cx.theme();
            let open = url.clone();
            let link = div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_family.clone())
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(theme.colors.fg_muted)
                        .child(MiddleEllipsis::new(url.clone())),
                )
                .child(CopyButton::new("add-copy-link", url.clone()))
                .child(tagged(
                    "add-open-again",
                    Button::new("add-open-again", tr(cx, keys::ADD_OPEN_AGAIN))
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| cx.open_url(&open)),
                ));
            let field = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(theme.text_size(TextSize::Sm))
                        .font_weight(FontWeight::MEDIUM)
                        .child(tr(cx, keys::ADD_CODE_LABEL)),
                )
                .child(tagged("add-code-field", Input::new(&inputs.code)));
            let (submit, code) = (handle.clone(), inputs.code.clone());
            let mut dialog = dialog
                .child(Paragraph::new(tr(cx, keys::ADD_OPENED)))
                .child(link)
                .child(field);
            if submitting {
                dialog = dialog.child(busy_line("add-submitting", tr(cx, keys::ADD_SUBMITTING)));
            }
            dialog
                .action({
                    let handle = handle.clone();
                    move |_| cancel_button(cancel_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "add-submit",
                        Button::new("add-submit", submit_label)
                            .primary()
                            .disabled(submitting)
                            .on_click(move |_, _, cx| {
                                let text = code.read(cx).text().trim().to_owned();
                                if text.is_empty() {
                                    return;
                                }
                                code.update(cx, |input, cx| input.set_text("", cx));
                                submit.update(cx, |state, cx| state.submit_code(text, cx));
                            }),
                    )
                })
        }
        AddPhase::Failed { error } => {
            let again = handle.clone();
            let close_label = tr(cx, keys::ACTION_CLOSE);
            let retry: error_view::Retry = {
                let handle = handle.clone();
                Rc::new(move |_, cx| handle.update(cx, |state, cx| state.begin_sign_in(cx)))
            };
            dialog
                .child(error_view::alert(
                    "add",
                    None,
                    error,
                    Some(retry),
                    handle,
                    cx,
                ))
                .child(Caption::new(tr(cx, keys::ADD_ROLLED_BACK)))
                .action({
                    let handle = handle.clone();
                    move |_| cancel_button(close_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "add-start-again",
                        Button::new("add-start-again", again_label)
                            .primary()
                            .on_click(move |_, _, cx| {
                                again.update(cx, |state, cx| state.begin_sign_in(cx))
                            }),
                    )
                })
        }
    };
    let _ = state;
    dialog.into_any_element()
}

fn switch(
    email: Option<&str>,
    impact: &Load<(SwitchImpact, SystemTime)>,
    error: Option<&colab_playground_core::Error>,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let email = email
        .map(str::to_owned)
        .unwrap_or_else(|| tr(cx, keys::ACCOUNTS_NO_EMAIL).to_string());
    let switching = state.is_busy(Op::Switch);
    let mut dialog = Dialog::new(
        "switch-account",
        tr_args(cx, keys::SWITCH_TITLE, &[("email", &email)]),
        close(handle),
    );
    let ready = matches!(impact, Load::Ready(_));
    dialog = match impact {
        Load::Idle | Load::Loading => {
            dialog.child(busy_line("switch-loading", tr(cx, keys::SWITCH_LOADING)))
        }
        Load::Failed(error) => dialog.child(error_view::alert(
            "switch-impact",
            None,
            error,
            None,
            handle,
            cx,
        )),
        Load::Ready((impact, now)) => dialog.child(impact_list(impact, *now, state, cx)),
    };
    if let Some(error) = error {
        dialog = dialog.child(error_view::alert("switch", None, error, None, handle, cx));
    }
    let confirm = handle.clone();
    let cancel_label = tr(cx, keys::ACTION_CANCEL);
    let confirm_label = tr(cx, keys::SWITCH_CONFIRM);
    dialog
        .action({
            let handle = handle.clone();
            move |_| cancel_button(cancel_label, handle)
        })
        .action(move |_| {
            tagged(
                "switch-confirm",
                Button::new("switch-confirm", confirm_label)
                    .primary()
                    .loading(switching)
                    .disabled(!ready || switching)
                    .on_click(move |_, _, cx| {
                        confirm.update(cx, |state, cx| state.confirm_switch(cx))
                    }),
            )
        })
        .into_any_element()
}

/// What switching affects: tunnels that close and runtimes that keep billing.
fn impact_list(impact: &SwitchImpact, now: SystemTime, state: &AppState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    if impact.tunnels_closing.is_empty()
        && impact.sessions_left_running.is_empty()
        && impact.unmanaged_count == 0
    {
        return Paragraph::new(tr(cx, keys::SWITCH_NOTHING)).into_any_element();
    }
    let item = |text: SharedString| {
        div()
            .pl_3()
            .text_size(theme.text_size(TextSize::Sm))
            .child(format!("• {text}"))
    };
    let mut list = div().flex().flex_col().gap_2();
    if !impact.tunnels_closing.is_empty() {
        list = list
            .child(Paragraph::new(tr(cx, keys::SWITCH_TUNNELS)))
            .children(impact.tunnels_closing.iter().map(|tunnel| {
                item(tr_args(
                    cx,
                    keys::SWITCH_TUNNEL,
                    &[
                        ("local", &tunnel.spec.local_port.to_string()),
                        ("session", &tunnel.spec.session),
                        ("remote", &tunnel.spec.remote_port.to_string()),
                    ],
                ))
            }));
    }
    if !impact.sessions_left_running.is_empty() {
        list = list
            .child(Paragraph::new(tr(cx, keys::SWITCH_SESSIONS)))
            .children(impact.sessions_left_running.iter().map(|session| {
                let hardware = tr(cx, format::accelerator_key(session.accelerator)).to_string();
                let deadline = session
                    .watchdog
                    .as_ref()
                    .and_then(|watchdog| watchdog.deadline)
                    .and_then(|deadline| deadline.duration_since(now).ok());
                item(match deadline {
                    Some(left) => tr_args(
                        cx,
                        keys::SWITCH_SESSION_DEADLINE,
                        &[
                            ("name", &session.name),
                            ("hardware", &hardware),
                            ("time", &format::duration(cx, left)),
                        ],
                    ),
                    None => tr_args(
                        cx,
                        keys::SWITCH_SESSION,
                        &[("name", &session.name), ("hardware", &hardware)],
                    ),
                })
            }));
    }
    if impact.unmanaged_count > 0 {
        // Runtimes opened in the browser or elsewhere keep billing with no automatic stop.
        list = list
            .child(Paragraph::new(tr(cx, keys::SWITCH_UNMANAGED)))
            .children(impact.unmanaged_left_running.iter().map(|runtime| {
                let label = state
                    .runtime(runtime)
                    .map(crate::state::runtime_label)
                    .or_else(|| runtime.name.clone())
                    .unwrap_or_else(|| runtime.endpoint.clone());
                item(tr_args(
                    cx,
                    keys::SWITCH_UNMANAGED_ITEM,
                    &[("name", &label)],
                ))
            }));
        let listed = impact.unmanaged_left_running.len() as u32;
        if impact.unmanaged_count > listed {
            list = list.child(item(tr_args(
                cx,
                keys::SWITCH_UNMANAGED_MORE,
                &[("count", &(impact.unmanaged_count - listed).to_string())],
            )));
        }
    }
    tagged("switch-impact", list).into_any_element()
}

fn remove(
    email: Option<&str>,
    error: Option<&colab_playground_core::Error>,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let email = email
        .map(str::to_owned)
        .unwrap_or_else(|| tr(cx, keys::ACCOUNTS_NO_EMAIL).to_string());
    let removing = state.is_busy(Op::Remove);
    let mut dialog = Dialog::new(
        "remove-account",
        tr_args(cx, keys::REMOVE_TITLE, &[("email", &email)]),
        close(handle),
    )
    .child(Paragraph::new(tr(cx, keys::REMOVE_BODY)));
    if let Some(error) = error {
        dialog = dialog.child(error_view::alert("remove", None, error, None, handle, cx));
    }
    let confirm = handle.clone();
    let cancel_label = tr(cx, keys::ACTION_CANCEL);
    let confirm_label = tr(cx, keys::REMOVE_CONFIRM);
    dialog
        .action({
            let handle = handle.clone();
            move |_| cancel_button(cancel_label, handle)
        })
        .action(move |_| {
            tagged(
                "remove-confirm",
                Button::new("remove-confirm", confirm_label)
                    .variant(ButtonVariant::Danger)
                    .loading(removing)
                    .disabled(removing)
                    .on_click(move |_, _, cx| {
                        confirm.update(cx, |state, cx| state.confirm_remove(cx))
                    }),
            )
        })
        .into_any_element()
}
