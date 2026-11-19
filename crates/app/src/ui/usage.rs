//! What more than one page says about compute-unit usage.

use std::time::Duration;

use colab_playground_core::{backend::AccountUsageView, usage::UsageSnapshot};
use ely_gpui_component::{
    buttons::Button, feedback::Alert, primitives::Severity, theme::ControlSize,
};
use gpui::{AnyElement, App, Entity, IntoElement, SharedString, Styled};

use crate::{
    format,
    i18n::{keys, tr, tr_args, tr_count},
    route::Route,
    state::AppState,
    ui::tagged,
};

/// When the balance runs out: the backend's estimate, else the one the snapshot gives.
/// `None` while nothing uses the balance up.
pub fn eta(usage: Option<&AccountUsageView>, snapshot: &UsageSnapshot) -> Option<Duration> {
    usage.and_then(|usage| usage.eta).or(snapshot.eta())
}

/// How long the balance lasts at the current rate; unknown while the account has no usage.
pub fn time_left(usage: Option<&AccountUsageView>, cx: &App) -> SharedString {
    let Some(snapshot) = usage.and_then(|usage| usage.snapshot) else {
        return tr(cx, keys::SUMMARY_UNKNOWN);
    };
    match eta(usage, &snapshot) {
        None => tr(cx, keys::SUMMARY_NOT_BURNING),
        Some(eta) if eta.is_zero() => tr(cx, keys::SUMMARY_EMPTY),
        Some(eta) => format::approx(cx, &format::duration(cx, eta)),
    }
}

/// Says that a total leaves `count` accounts out.
pub fn accounts_missing(count: usize, cx: &App) -> SharedString {
    tr_count(
        cx,
        count,
        keys::ACCOUNTS_TOTAL_MISSING_ONE,
        keys::ACCOUNTS_TOTAL_MISSING_MANY,
        &[],
    )
}

/// Warns about runtimes the app did not start that are billing.
pub fn unmanaged_warning(
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> Option<AnyElement> {
    let unmanaged = state.unmanaged();
    if unmanaged.is_empty() {
        return None;
    }
    // A row with a count is the stronger news, so rows of unknown ownership only decide the
    // title when there is nothing else.
    let mut counted = unmanaged
        .iter()
        .filter(|(_, count, _)| count.is_some())
        .peekable();
    let title = if counted.peek().is_none() {
        keys::UNMANAGED_UNKNOWN_TITLE
    } else if counted.all(|(_, _, stale)| *stale) {
        keys::UNMANAGED_LAST_TITLE
    } else {
        keys::UNMANAGED_TITLE
    };
    let lines: Vec<String> = unmanaged
        .iter()
        .map(|(email, count, stale)| {
            let email = format::email(cx, email.as_deref());
            let Some(count) = count else {
                return tr_args(cx, keys::UNMANAGED_UNKNOWN, &[("email", &email)]).to_string();
            };
            let (one, many) = if *stale {
                (keys::UNMANAGED_LAST_ONE, keys::UNMANAGED_LAST_MANY)
            } else {
                (keys::UNMANAGED_ONE, keys::UNMANAGED_MANY)
            };
            tr_count(cx, *count as usize, one, many, &[("email", &email)]).to_string()
        })
        .collect();
    let handle = handle.clone();
    let body = lines.join("\n");
    Some(
        tagged(
            "unmanaged-warning",
            tagged(
                format!("unmanaged-warning {} {body}", tr(cx, title)),
                Alert::new("unmanaged", Severity::Warning, tr(cx, title))
                    .body(body)
                    .action(
                        Button::new("unmanaged-view", tr(cx, keys::ACTION_VIEW_SESSIONS))
                            .size(ControlSize::Sm)
                            .on_click(move |_, _, cx| {
                                handle.update(cx, |state, cx| state.navigate(Route::Sessions, cx))
                            }),
                    ),
            ),
        )
        .w_full()
        .into_any_element(),
    )
}
