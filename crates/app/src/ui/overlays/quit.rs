//! Quitting with runtimes still billing: stop them all, leave them running (saying which
//! have an automatic stop and which do not), or cancel.

use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    data_display::{Badge, Tone},
    feedback::InlineMessage,
    overlays::Dialog,
    primitives::Severity,
    theme::{ActiveTheme, TextSize},
    typography::Paragraph,
};
use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, SharedString, Styled, div};

use super::{busy_line, cancel_button_when, close};
use crate::{
    errors,
    i18n::{keys, tr, tr_args},
    state::{AppState, QuitPrompt},
    ui::tagged,
};

pub const QUIT_STOP_ALL: &str = "quit-stop-all";
pub const QUIT_LEAVE: &str = "quit-leave";
/// Selector of the line saying the app is still checking what runs.
pub const QUIT_CHECKING: &str = "quit-checking";
/// Selector of the line saying the app could not check every account.
pub const QUIT_UNCHECKED: &str = "quit-unchecked";

fn count(cx: &App, one: &str, many: &str, n: usize) -> SharedString {
    if n == 1 {
        tr(cx, one)
    } else {
        tr_args(cx, many, &[("count", &n.to_string())])
    }
}

pub(super) fn render(
    prompt: &QuitPrompt,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    // Live, not the snapshot taken when the prompt opened: the list may still be loading.
    let live = state.billing();
    let billing = &live;
    let list_failed = matches!(state.runtimes(), crate::state::Load::Failed(_));
    let stopping = state.is_stopping_all();
    // A prompt that waits with nothing to show is waiting for a listing that refreshes the
    // list shown, which the list itself does not say.
    let checking = billing.checking || (prompt.waiting && !billing.any());
    let title = if billing.known() {
        keys::QUIT_TITLE
    } else if checking {
        keys::QUIT_TITLE_CHECKING
    } else if billing.unknown {
        keys::QUIT_TITLE_UNCHECKED
    } else {
        keys::QUIT_TITLE
    };
    let mut lines: Vec<(Severity, SharedString)> = Vec::new();
    if !billing.watched.is_empty() {
        lines.push((
            Severity::Info,
            count(
                cx,
                keys::QUIT_WATCHED_ONE,
                keys::QUIT_WATCHED_MANY,
                billing.watched.len(),
            ),
        ));
    }
    if !billing.unwatched.is_empty() {
        lines.push((
            Severity::Warning,
            count(
                cx,
                keys::QUIT_UNWATCHED_ONE,
                keys::QUIT_UNWATCHED_MANY,
                billing.unwatched.len(),
            ),
        ));
    }
    if billing.provisioning {
        lines.push((Severity::Warning, tr(cx, keys::QUIT_PROVISIONING)));
    }
    if billing.running_plugin {
        lines.push((Severity::Warning, tr(cx, keys::QUIT_RUNNING_PLUGIN)));
    }
    let unchecked = billing.unknown.then(|| {
        let text = if list_failed {
            keys::QUIT_UNCHECKED
        } else {
            keys::QUIT_UNKNOWN
        };
        tagged(
            QUIT_UNCHECKED,
            InlineMessage::new(Severity::Warning, tr(cx, text)),
        )
    });
    // While checking. A prompt waiting on the check closes by itself if it finds nothing; with
    // nothing known to bill yet, the line says so.
    let checking_line = checking.then(|| {
        let text = if prompt.waiting && !billing.known() {
            keys::QUIT_CHECKING
        } else {
            keys::SESSIONS_LOADING
        };
        tagged(
            QUIT_CHECKING,
            InlineMessage::new(Severity::Info, tr(cx, text)),
        )
    });
    let mut dialog = Dialog::new("quit", tr(cx, title), close(handle))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children(
                    lines
                        .into_iter()
                        .map(|(severity, text)| InlineMessage::new(severity, text)),
                )
                .children(unchecked)
                .children(checking_line),
        )
        .child(runtime_list(billing, state, cx))
        .child(Paragraph::new(tr(cx, keys::QUIT_LEAVE_EXPLAINED)))
        .child(tagged(
            "quit-wipe",
            InlineMessage::new(Severity::Warning, tr(cx, keys::QUIT_STOP_WIPED)),
        ));
    for (label, error) in &prompt.failures {
        let message = errors::explain(error, false, cx).message;
        // No label: "Stop all" failed before it got to any runtime (listing them failed).
        let text = if label.is_empty() {
            tr_args(cx, keys::QUIT_STOP_ALL_FAILED, &[("why", &message)])
        } else {
            tr_args(
                cx,
                keys::QUIT_STOP_FAILED,
                &[("name", label), ("why", &message)],
            )
        };
        dialog = dialog.child(tagged(
            "quit-failure",
            InlineMessage::new(Severity::Danger, text),
        ));
    }
    if stopping {
        dialog = dialog.child(busy_line("quit-stopping", tr(cx, keys::QUIT_STOPPING)));
    }
    let (cancel, leave, stop) = (
        tr(cx, keys::ACTION_CANCEL),
        tr(cx, keys::QUIT_LEAVE),
        tr(cx, keys::QUIT_STOP_ALL),
    );
    let (leave_handle, stop_handle) = (handle.clone(), handle.clone());
    // "Stop all" lists afresh before stopping, so it is offered once the check is over, even
    // when it failed.
    let still_checking = checking;
    dialog
        .action({
            let handle = handle.clone();
            move |_| cancel_button_when(cancel, handle, !stopping)
        })
        .action(move |_| {
            tagged(
                QUIT_LEAVE,
                Button::new(QUIT_LEAVE, leave)
                    .disabled(stopping)
                    .on_click(move |_, _, cx| {
                        leave_handle.update(cx, |state, cx| state.quit_leaving_running(cx))
                    }),
            )
        })
        .action(move |_| {
            tagged(
                QUIT_STOP_ALL,
                Button::new(QUIT_STOP_ALL, stop)
                    .variant(ButtonVariant::Danger)
                    .loading(stopping)
                    .disabled(stopping || still_checking)
                    .on_click(move |_, _, cx| {
                        stop_handle.update(cx, |state, cx| state.quit_stopping_all(cx))
                    }),
            )
        })
        .into_any_element()
}

/// Every runtime "Stop all" would stop, by name and account, marking the ones this app did
/// not start.
fn runtime_list(billing: &crate::state::Billing, state: &AppState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let rows = billing
        .watched
        .iter()
        .chain(&billing.unwatched)
        .filter_map(|target| state.runtime(target))
        .map(|runtime| {
            let label = crate::state::runtime_label(runtime);
            let email = runtime
                .account_email
                .clone()
                .unwrap_or_else(|| tr(cx, keys::ACCOUNTS_NO_EMAIL).to_string());
            let mut row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .text_size(theme.text_size(TextSize::Sm))
                .child(format!(
                    "• {}",
                    tr_args(cx, keys::ENDED_LINE, &[("name", &label), ("email", &email)])
                ));
            if !runtime.managed {
                row = row.child(tagged(
                    format!("quit-unmanaged {label}"),
                    Badge::new(tr(cx, keys::RUNTIME_UNMANAGED)).tone(Tone::Warning),
                ));
            }
            tagged(format!("quit-runtime {label}"), row)
        });
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(rows)
        .into_any_element()
}
