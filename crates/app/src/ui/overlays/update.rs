//! Updating in place: the confirmation, saying what a restart affects; the live progress with
//! the steps still to come and Cancel; and what went wrong. The body carries the selector
//! `update-phase <name>`.

use colab_playground_core::{
    ErrorKind,
    updater::{Unsupported, UpdateProgress},
};
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    feedback::InlineMessage,
    forms::Choice,
    motion::ProgressBar,
    navigation::Steps,
    overlays::Dialog,
    primitives::{IconName, Severity},
    typography::{Caption, Paragraph},
};
use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, SharedString, Styled, div};

use super::{cancel_button, close};
use crate::{
    errors, format,
    i18n::{keys, tr, tr_args},
    state::{AppState, Op, UpdatePhase, UpdatePrompt},
    ui::tagged,
};

pub const UPDATE_CONFIRM: &str = "update-confirm";
pub const UPDATE_CANCEL: &str = "update-cancel";
pub const UPDATE_RETRY: &str = "update-retry";
pub const UPDATE_OPEN_PAGE: &str = "update-open-page";
pub const UPDATE_QUIT: &str = "update-quit";

// Positions in the row of steps.
const STEP_DOWNLOAD: usize = 0;
const STEP_CHECK: usize = 1;
const STEP_INSTALL: usize = 2;
const STEP_RESTART: usize = 3;

/// Why this copy cannot replace itself, as a sentence.
pub fn unsupported_reason(reason: &Unsupported, cx: &App) -> SharedString {
    match reason {
        Unsupported::DevelopmentBuild => tr(cx, keys::UPDATE_WHY_DEVELOPMENT),
        Unsupported::NoDownload => tr(cx, keys::UPDATE_WHY_NO_DOWNLOAD),
        Unsupported::NotWritable { folder } => {
            tr_args(cx, keys::UPDATE_WHY_NOT_WRITABLE, &[("folder", folder)])
        }
        Unsupported::Translocated => tr(cx, keys::UPDATE_WHY_TRANSLOCATED),
        Unsupported::NotInBundle => tr(cx, keys::UPDATE_WHY_NOT_IN_BUNDLE),
        Unsupported::UnexpectedLayout => tr(cx, keys::UPDATE_WHY_LAYOUT),
    }
}

pub(super) fn render(
    prompt: &UpdatePrompt,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    match &prompt.phase {
        UpdatePhase::Confirm => confirm(prompt, state, handle, cx),
        UpdatePhase::Working(_) | UpdatePhase::Restarting => working(prompt, state, handle, cx),
        UpdatePhase::Failed(error) => failed(prompt, error, handle, cx),
        UpdatePhase::RestartFailed(error) => restart_failed(prompt, error, handle, cx),
    }
}

fn confirm(
    prompt: &UpdatePrompt,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let version = [("version", prompt.version.as_str())];
    let block = state.update_block();
    let consequences = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(tagged(
            "update-consequence forwards",
            InlineMessage::new(Severity::Warning, tr(cx, keys::UPDATE_CONSEQUENCE_FORWARDS)),
        ))
        .child(tagged(
            "update-consequence billing",
            InlineMessage::new(Severity::Info, tr(cx, keys::UPDATE_CONSEQUENCE_BILLING)),
        ))
        .child(tagged(
            "update-consequence auto-stop",
            InlineMessage::new(Severity::Info, tr(cx, keys::UPDATE_CONSEQUENCE_AUTO_STOP)),
        ));
    let dialog = Dialog::new(
        "update",
        tr_args(cx, keys::UPDATE_CONFIRM_TITLE, &version),
        close(handle),
    )
    .child(tagged(
        "update-phase confirm",
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Paragraph::new(tr_args(
                cx,
                keys::UPDATE_CONFIRM_BODY,
                &version,
            )))
            .child(consequences)
            .children(block.map(|block| {
                tagged(
                    "update-blocked",
                    InlineMessage::new(Severity::Warning, tr(cx, block)),
                )
            })),
    ));
    let (cancel, confirm) = (tr(cx, keys::ACTION_CANCEL), tr(cx, keys::UPDATE_INSTALL));
    let go = handle.clone();
    dialog
        .action({
            let handle = handle.clone();
            move |_| cancel_button(cancel, handle)
        })
        .action(move |_| {
            tagged(
                UPDATE_CONFIRM,
                Button::new(UPDATE_CONFIRM, confirm)
                    .primary()
                    .icon(IconName::Download)
                    .disabled(block.is_some())
                    .on_click(move |_, _, cx| go.update(cx, |state, cx| state.confirm_update(cx))),
            )
        })
        .into_any_element()
}

/// What the dialog shows while it works.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressLine {
    /// The step on show: download, check, install, restart.
    pub step: usize,
    /// Name in the selector `update-phase <name>`.
    pub name: &'static str,
    /// What happens now.
    pub text: SharedString,
    /// How much of the download has arrived (0 to 1), when the size is known.
    pub share: Option<f32>,
}

/// The [`ProgressLine`] for `phase`; `waiting` says a restart waits for a call to finish.
pub fn progress_line(phase: &UpdatePhase, waiting: bool, cx: &App) -> ProgressLine {
    let (step, name, text, share) = progress(phase, waiting, cx);
    ProgressLine {
        step,
        name,
        text,
        share,
    }
}

fn progress(
    phase: &UpdatePhase,
    waiting: bool,
    cx: &App,
) -> (usize, &'static str, SharedString, Option<f32>) {
    match phase {
        UpdatePhase::Working(UpdateProgress::Preparing) => (
            STEP_DOWNLOAD,
            "preparing",
            tr(cx, keys::UPDATE_PREPARING),
            None,
        ),
        UpdatePhase::Working(UpdateProgress::Downloading { received, total }) => {
            let received_text = format::bytes(cx, *received);
            let (text, share) = match total.filter(|total| *total > 0) {
                Some(total) => (
                    tr_args(
                        cx,
                        keys::UPDATE_DOWNLOADING,
                        &[
                            ("received", &received_text),
                            ("total", &format::bytes(cx, total)),
                        ],
                    ),
                    Some((*received as f64 / total as f64).clamp(0.0, 1.0) as f32),
                ),
                None => (
                    tr_args(
                        cx,
                        keys::UPDATE_DOWNLOADING_UNSIZED,
                        &[("received", &received_text)],
                    ),
                    None,
                ),
            };
            (STEP_DOWNLOAD, "downloading", text, share)
        }
        UpdatePhase::Working(UpdateProgress::Verifying) => (
            STEP_CHECK,
            "verifying",
            tr(cx, keys::UPDATE_VERIFYING),
            None,
        ),
        UpdatePhase::Working(UpdateProgress::Installing) => (
            STEP_INSTALL,
            "installing",
            tr(cx, keys::UPDATE_INSTALLING),
            None,
        ),
        _ if waiting => (
            STEP_RESTART,
            "waiting",
            tr(cx, keys::UPDATE_RESTART_WAITING),
            None,
        ),
        _ => (
            STEP_RESTART,
            "restarting",
            tr(cx, keys::UPDATE_RESTARTING),
            None,
        ),
    }
}

fn working(
    prompt: &UpdatePrompt,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let ProgressLine {
        step,
        name,
        text: line,
        share,
    } = progress_line(&prompt.phase, state.restart_waiting(), cx);
    let cancellable = state.update_cancellable();
    let cancelling = state.is_cancelling(Op::UpdateInstall);
    let line = if cancelling {
        tr(cx, keys::UPDATE_CANCELLING)
    } else {
        line
    };
    let steps = Steps::new(
        "update-steps",
        [
            Choice::new("download", tr(cx, keys::UPDATE_STEP_DOWNLOAD)),
            Choice::new("check", tr(cx, keys::UPDATE_STEP_CHECK)),
            Choice::new("install", tr(cx, keys::UPDATE_STEP_INSTALL)),
            Choice::new("restart", tr(cx, keys::UPDATE_STEP_RESTART)),
        ],
        step,
    );
    let bar = match share {
        Some(share) => ProgressBar::new("update-progress", share),
        None => ProgressBar::indeterminate("update-progress"),
    };
    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(tagged("update-steps", steps))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(Paragraph::new(line))
                .child(bar),
        )
        .children(
            (cancellable && !cancelling).then(|| Caption::new(tr(cx, keys::UPDATE_CANCEL_NOTE))),
        );
    let cancel = handle.clone();
    let label = tr(cx, keys::ACTION_CANCEL);
    Dialog::new(
        "update",
        tr_args(
            cx,
            keys::UPDATE_WORKING_TITLE,
            &[("version", &prompt.version)],
        ),
        close(handle),
    )
    .child(tagged(format!("update-phase {name}"), body))
    .action(move |_| {
        tagged(
            UPDATE_CANCEL,
            Button::new(UPDATE_CANCEL, label)
                .variant(ButtonVariant::Ghost)
                .loading(cancelling)
                .disabled(!cancellable || cancelling)
                .on_click(move |_, _, cx| cancel.update(cx, |state, cx| state.cancel_update(cx))),
        )
    })
    .into_any_element()
}

fn failed(
    _prompt: &UpdatePrompt,
    error: &colab_playground_core::Error,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let explained = errors::explain(error, false, cx);
    let (name, message) = match error.kind {
        ErrorKind::UpdateUnverified => ("unverified", tr(cx, keys::UPDATE_UNVERIFIED)),
        ErrorKind::UpdateUnavailable => ("unavailable", tr(cx, keys::UPDATE_UNAVAILABLE)),
        _ => (
            "error",
            tr_args(
                cx,
                keys::UPDATE_INSTALL_FAILED,
                &[("why", &explained.message)],
            ),
        ),
    };
    // A download that failed its check is never offered again by hand: that would skip the
    // check. Trying again fetches the same files only when the failure was elsewhere.
    let unverified = error.is(ErrorKind::UpdateUnverified);
    let can_retry = !unverified && !error.is(ErrorKind::UpdateUnavailable);
    let mut dialog = Dialog::new(
        "update",
        tr(cx, keys::UPDATE_FAILED_INSTALL_TITLE),
        close(handle),
    )
    .child(tagged(
        format!("update-phase failed {name}"),
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(InlineMessage::new(Severity::Danger, message))
            .children(explained.detail.map(Caption::new)),
    ));
    let close_label = tr(cx, keys::ACTION_CLOSE);
    dialog = dialog.action({
        let handle = handle.clone();
        move |_| cancel_button(close_label, handle)
    });
    if !unverified {
        let open = handle.clone();
        let label = tr(cx, keys::UPDATE_OPEN_PAGE);
        dialog = dialog.action(move |_| {
            tagged(
                UPDATE_OPEN_PAGE,
                Button::new(UPDATE_OPEN_PAGE, label)
                    .icon(IconName::ExternalLink)
                    .on_click(move |_, _, cx| {
                        open.update(cx, |state, cx| state.open_update_page(cx))
                    }),
            )
        });
    }
    if can_retry {
        let retry = handle.clone();
        let label = tr(cx, keys::UPDATE_TRY_AGAIN);
        dialog = dialog.action(move |_| {
            tagged(
                UPDATE_RETRY,
                Button::new(UPDATE_RETRY, label)
                    .primary()
                    .on_click(move |_, _, cx| {
                        retry.update(cx, |state, cx| state.confirm_update(cx))
                    }),
            )
        });
    }
    dialog.into_any_element()
}

fn restart_failed(
    prompt: &UpdatePrompt,
    error: &colab_playground_core::Error,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let explained = errors::explain(error, false, cx);
    let message = tr_args(
        cx,
        keys::UPDATE_RESTART_FAILED,
        &[("version", &prompt.version), ("why", &explained.message)],
    );
    let (close_label, quit_label) = (tr(cx, keys::ACTION_CLOSE), tr(cx, keys::UPDATE_QUIT));
    let quit = handle.clone();
    Dialog::new(
        "update",
        tr(cx, keys::UPDATE_RESTART_FAILED_TITLE),
        close(handle),
    )
    .child(tagged(
        "update-phase restart-failed",
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(InlineMessage::new(Severity::Warning, message))
            .children(explained.detail.map(Caption::new)),
    ))
    .action({
        let handle = handle.clone();
        move |_| cancel_button(close_label, handle)
    })
    .action(move |_| {
        tagged(
            UPDATE_QUIT,
            Button::new(UPDATE_QUIT, quit_label)
                .primary()
                .on_click(move |_, _, cx| quit.update(cx, |state, cx| state.request_quit(cx))),
        )
    })
    .into_any_element()
}
