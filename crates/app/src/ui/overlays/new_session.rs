//! New session: the form, then live progress, then ready / failed / cancelled. Closing while
//! it provisions only hides the dialog (the Sessions page keeps the progress and Cancel).

use std::rc::Rc;

use colab_playground_core::ErrorKind;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    feedback::InlineMessage,
    forms::Input,
    overlays::Dialog,
    primitives::{IconName, Severity},
    theme::{ActiveTheme, TextSize},
    typography::{Caption, Paragraph},
};
use gpui::{AnyElement, App, Entity, FontWeight, IntoElement, ParentElement, Styled, div};

use super::{cancel_button, close, scrolled};
use crate::{
    i18n::{keys, tr, tr_args},
    route::Route,
    state::{AppState, ProvisionPhase},
    ui::{Inputs, error_view, progress, provision_form, tagged},
};

pub const NEW_SESSION_START: &str = "new-session-start";
/// Selector of Cancel while provisioning.
pub const NEW_SESSION_CANCEL_RUN: &str = "new-session-cancel-run";

pub(super) fn render(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> Option<AnyElement> {
    let session = state.new_session()?;
    let dialog = Dialog::new("new-session", tr(cx, keys::NEW_TITLE), close(handle));
    let theme = cx.theme();
    Some(match &session.phase {
        ProvisionPhase::Editing if state.session_limit_reached().is_some() => {
            limit_dialog(dialog, state, handle, cx)
        }
        ProvisionPhase::Editing => {
            let edit: provision_form::SpecEdit = {
                let handle = handle.clone();
                Rc::new(move |cx: &mut App, f: &dyn Fn(&mut colab_playground_core::provision::ProvisionSpec)| {
                    handle.update(cx, |state, cx| state.edit_new_session(|spec| f(spec), cx))
                })
            };
            let blocked = state.is_provisioning();
            let form = div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(theme.text_size(TextSize::Sm))
                                .font_weight(FontWeight::MEDIUM)
                                .child(tr(cx, keys::NEW_NAME)),
                        )
                        .child(tagged("new-session-name", Input::new(&inputs.session_name))),
                )
                .child(provision_form::render("new", &session.spec, edit, cx));
            let (cancel, start) = (tr(cx, keys::ACTION_CANCEL), tr(cx, keys::NEW_START));
            let (go, name) = (handle.clone(), inputs.session_name.clone());
            dialog
                .child(Paragraph::new(tr(cx, keys::NEW_INTRO)))
                .child(scrolled("new-session-form", form))
                .action({
                    let handle = handle.clone();
                    move |_| cancel_button(cancel, handle)
                })
                .action(move |_| {
                    tagged(
                        NEW_SESSION_START,
                        Button::new(NEW_SESSION_START, start)
                            .primary()
                            .icon(IconName::Play)
                            .disabled(blocked)
                            .on_click(move |_, _, cx| {
                                let name = name.read(cx).text().to_owned();
                                go.update(cx, |state, cx| state.start_provision(name, cx))
                            }),
                    )
                })
                .into_any_element()
        }
        ProvisionPhase::Running => {
            let cancelling = state.is_cancelling_provision();
            let (hide, cancel) = (
                tr(cx, keys::ACTION_HIDE),
                tr(
                    cx,
                    if cancelling {
                        keys::NEW_CANCELLING
                    } else {
                        keys::ACTION_CANCEL
                    },
                ),
            );
            let (hide_handle, cancel_handle) = (handle.clone(), handle.clone());
            dialog
                .child(
                    div()
                        .text_size(theme.text_size(TextSize::Sm))
                        .font_weight(FontWeight::MEDIUM)
                        .child(progress::headline(&session.progress, cx)),
                )
                .child(progress::render(&session.progress, true, cx))
                .child(InlineMessage::new(
                    Severity::Info,
                    tr(cx, keys::NEW_CANCEL_NOTE),
                ))
                .action(move |_| {
                    tagged(
                        "new-session-hide",
                        Button::new("new-session-hide", hide)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                hide_handle.update(cx, |state, cx| state.close_overlay(cx))
                            }),
                    )
                })
                .action(move |_| {
                    tagged(
                        NEW_SESSION_CANCEL_RUN,
                        Button::new(NEW_SESSION_CANCEL_RUN, cancel)
                            .variant(ButtonVariant::Danger)
                            .loading(cancelling)
                            .disabled(cancelling)
                            .on_click(move |_, _, cx| {
                                cancel_handle.update(cx, |state, cx| state.cancel_provision(cx))
                            }),
                    )
                })
                .into_any_element()
        }
        ProvisionPhase::Ready(outcome) => {
            let (close_label, show) = (tr(cx, keys::ACTION_CLOSE), tr(cx, keys::NEW_SHOW_SESSIONS));
            let show_handle = handle.clone();
            dialog
                .child(progress::render(&session.progress, false, cx))
                .child(InlineMessage::new(
                    Severity::Success,
                    tr_args(cx, keys::NEW_READY, &[("name", &outcome.name)]),
                ))
                .action({
                    let handle = handle.clone();
                    move |_| cancel_button(close_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "new-session-show",
                        Button::new("new-session-show", show).primary().on_click(
                            move |_, _, cx| {
                                show_handle.update(cx, |state, cx| {
                                    state.close_overlay(cx);
                                    state.navigate(Route::Sessions, cx);
                                })
                            },
                        ),
                    )
                })
                .into_any_element()
        }
        ProvisionPhase::Failed(_) | ProvisionPhase::Cancelled => {
            let failed = match &session.phase {
                ProvisionPhase::Failed(error) => Some(error),
                _ => None,
            };
            if failed.is_some_and(|e| e.is(ErrorKind::SessionLimitReached)) {
                // The cap is this app's own setting: offer to change it or see what counts.
                return Some(limit_dialog(dialog, state, handle, cx));
            }
            let mut dialog = dialog;
            if progress::has_lines(&session.progress, cx) {
                dialog = dialog.child(progress::render(&session.progress, false, cx));
            }
            dialog = match failed {
                Some(error) => dialog.child(error_view::alert(
                    "provision",
                    Some(tr(cx, keys::NEW_FAILED)),
                    error,
                    None,
                    handle,
                    cx,
                )),
                None => dialog.child(tagged(
                    "provision-cancelled",
                    InlineMessage::new(Severity::Info, tr(cx, keys::NEW_CANCELLED)),
                )),
            };
            let (close_label, again) = (tr(cx, keys::ACTION_CLOSE), tr(cx, keys::NEW_CHANGE));
            let again_handle = handle.clone();
            dialog
                .action({
                    let handle = handle.clone();
                    move |_| cancel_button(close_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "new-session-change",
                        Button::new("new-session-change", again).primary().on_click(
                            move |_, _, cx| {
                                again_handle.update(cx, |state, cx| state.edit_again(cx))
                            },
                        ),
                    )
                })
                .into_any_element()
        }
    })
}

pub const NEW_SESSION_LIMIT: &str = "new-session-limit";

/// The session cap is reached: what counts, that it is this app's own setting, and the way to
/// change it or to see the runtimes that count.
fn limit_dialog(
    dialog: Dialog,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let limit = state.session_limit_reached();
    let max = limit
        .as_ref()
        .map_or(state.defaults().max_concurrent_sessions, |l| l.limit);
    let email = state
        .active_email()
        .unwrap_or_else(|| tr(cx, keys::ACCOUNTS_NO_EMAIL).to_string());
    let counted = limit
        .as_ref()
        .map(|l| crate::format::list(cx, &l.counted))
        .unwrap_or_default();
    let mut body = div().flex().flex_col().gap_2().child(InlineMessage::new(
        Severity::Warning,
        tr_args(
            cx,
            keys::NEW_LIMIT_BODY,
            &[("limit", &max.to_string()), ("email", &email)],
        ),
    ));
    if !counted.is_empty() {
        body = body.child(Paragraph::new(tr_args(
            cx,
            keys::NEW_LIMIT_COUNTED,
            &[("names", &counted)],
        )));
    }
    body = body.child(Caption::new(tr(cx, keys::NEW_LIMIT_HINT)));
    let (close_label, sessions, settings) = (
        tr(cx, keys::ACTION_CLOSE),
        tr(cx, keys::NEW_SHOW_SESSIONS),
        tr(cx, keys::NEW_OPEN_SETTINGS),
    );
    let (sessions_handle, settings_handle) = (handle.clone(), handle.clone());
    dialog
        .child(tagged(NEW_SESSION_LIMIT, body))
        .action({
            let handle = handle.clone();
            move |_| cancel_button(close_label, handle)
        })
        .action(move |_| {
            tagged(
                "new-session-open-sessions",
                Button::new("new-session-open-sessions", sessions).on_click(move |_, _, cx| {
                    sessions_handle.update(cx, |state, cx| {
                        state.close_overlay(cx);
                        state.navigate(Route::Sessions, cx);
                    })
                }),
            )
        })
        .action(move |_| {
            tagged(
                "new-session-open-settings",
                Button::new("new-session-open-settings", settings)
                    .primary()
                    .on_click(move |_, _, cx| {
                        settings_handle.update(cx, |state, cx| state.open_limit_setting(cx))
                    }),
            )
        })
        .into_any_element()
}
