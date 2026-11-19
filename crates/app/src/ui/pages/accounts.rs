//! Accounts: every saved Google account with its usage, totals, adding, switching, removing,
//! and the states around them (CLI missing, keyring unavailable, an unsaved CLI account, an
//! interrupted sign-in).

use colab_playground_core::backend::AccountUsageView;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant, IconButton},
    data_display::{Badge, Tone},
    feedback::{Alert, EmptyState, InlineMessage},
    layout::{Card, CardHeader, Page},
    primitives::{IconName, Severity, Tooltip},
    theme::{ActiveTheme, ControlSize, TextSize},
    typography::Caption,
};
use gpui::{
    AnyElement, App, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div,
};

use crate::{
    errors::explain,
    format,
    i18n::{keys, tr, tr_args},
    route::Route,
    state::{AccountRow, AppState, Op, RemoveBlock, SignInStatus, usage_is_stale},
    ui::{
        error_view, metrics,
        parts::{busy_line, captioned, figure},
        tagged,
        usage::{self, unmanaged_warning},
    },
};

/// Selector around "Add account" in the page header.
pub const ADD_SELECTOR: &str = "accounts-add";
/// Selector around "Use this account".
pub const ADOPT_SELECTOR: &str = "accounts-adopt";
/// Selector around the usage refresh button.
pub const REFRESH_SELECTOR: &str = "accounts-refresh";
pub const TOTALS_SELECTOR: &str = "accounts-totals";
/// Selector of the note shown when a whole usage refresh failed.
pub const USAGE_ERROR_SELECTOR: &str = "accounts-usage-error";

/// The selector of an account row: `account <email>` (plus ` active` for the active one).
pub fn row_selector(email: &str, active: bool) -> String {
    if active {
        format!("account {email} active")
    } else {
        format!("account {email}")
    }
}

pub fn switch_selector(email: &str) -> String {
    format!("account-switch {email}")
}

pub fn remove_selector(email: &str) -> String {
    format!("account-remove {email}")
}

pub fn render(state: &AppState, handle: &Entity<AppState>, cx: &App) -> Page {
    let status = state.sign_in_status();
    let can_add = matches!(
        status,
        SignInStatus::SignedIn { .. } | SignInStatus::SignedOut | SignInStatus::Unadopted { .. }
    );
    let add = handle.clone();
    let page = super::page(Route::Accounts, keys::ACCOUNTS_SUBTITLE, cx).action(tagged(
        ADD_SELECTOR,
        Button::new("accounts-add", tr(cx, keys::ACTION_ADD_ACCOUNT))
            .icon(IconName::UserPlus)
            .primary()
            .disabled(!can_add)
            .on_click(move |_, _, cx| add.update(cx, |state, cx| state.open_add_account(cx))),
    ));
    match status {
        SignInStatus::Checking => page.child(busy_line(
            "accounts-loading",
            tr(cx, keys::ACCOUNTS_LOADING),
        )),
        SignInStatus::NeedsCli => {
            let go = handle.clone();
            page.child(
                EmptyState::new(
                    "accounts-needs-cli",
                    IconName::Terminal,
                    tr(cx, keys::ERR_NOT_INSTALLED),
                )
                .body(tr(cx, keys::ACCOUNTS_NEEDS_CLI))
                .action(
                    Button::new("accounts-install", tr(cx, keys::ACTION_INSTALL_CLI)).on_click(
                        move |_, _, cx| {
                            go.update(cx, |state, cx| state.navigate(Route::Overview, cx))
                        },
                    ),
                ),
            )
        }
        SignInStatus::KeyringUnavailable(_) => {
            let again = handle.clone();
            page.child(tagged(
                "accounts-keyring",
                Alert::new(
                    "keyring",
                    Severity::Danger,
                    tr(cx, keys::ACCOUNTS_KEYRING_TITLE),
                )
                .body(tr(cx, keys::ACCOUNTS_KEYRING_BODY))
                .action(tagged(
                    "keyring-again",
                    Button::new("keyring-again", tr(cx, keys::ACTION_CHECK_AGAIN))
                        .size(ControlSize::Sm)
                        .on_click(move |_, _, cx| {
                            again.update(cx, |state, cx| state.check_environment(cx))
                        }),
                )),
            ))
        }
        SignInStatus::Failed(error) => {
            let retry = error_view::retry(handle, AppState::load_accounts);
            page.child(error_view::alert(
                "accounts",
                None,
                &error,
                Some(retry),
                handle,
                cx,
            ))
        }
        SignInStatus::SignedOut
        | SignInStatus::Unadopted { .. }
        | SignInStatus::SignedIn { .. } => listing(page, &status, state, handle, cx),
    }
}

fn listing(
    page: Page,
    status: &SignInStatus,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> Page {
    let mut page = page;
    if state.recovered_notice() {
        let dismiss = handle.clone();
        page = page.child(tagged(
            "accounts-recovered",
            Alert::new(
                "recovered",
                Severity::Info,
                tr(cx, keys::ACCOUNTS_RECOVERED_TITLE),
            )
            .body(tr(cx, keys::ACCOUNTS_RECOVERED_BODY))
            .on_dismiss(move |_, cx| {
                dismiss.update(cx, |state, cx| state.dismiss_recovered_notice(cx))
            }),
        ));
    }
    if let SignInStatus::Unadopted { email } = status {
        let adopt = handle.clone();
        let title = match email {
            Some(email) => tr_args(cx, keys::ACCOUNTS_UNADOPTED_TITLE, &[("email", email)]),
            None => tr(cx, keys::ACCOUNTS_UNADOPTED_TITLE_NO_EMAIL),
        };
        page = page.child(tagged(
            "accounts-unadopted",
            Alert::new("unadopted", Severity::Info, title)
                .body(tr(cx, keys::ACCOUNTS_UNADOPTED_BODY))
                .action(tagged(
                    ADOPT_SELECTOR,
                    Button::new("accounts-adopt", tr(cx, keys::ACCOUNTS_ADOPT))
                        .primary()
                        .size(ControlSize::Sm)
                        .disabled(state.is_busy(Op::Adopt))
                        .on_click(move |_, _, cx| {
                            adopt.update(cx, |state, cx| state.adopt_account(cx))
                        }),
                )),
        ));
    }
    let rows = state.account_rows();
    if rows.is_empty() {
        let add = handle.clone();
        return page.child(
            EmptyState::new(
                "accounts-empty",
                IconName::Users,
                tr(cx, keys::ACCOUNTS_EMPTY_TITLE),
            )
            .body(tr(cx, keys::ACCOUNTS_EMPTY_BODY))
            .action(
                Button::new("accounts-empty-add", tr(cx, keys::ACTION_ADD_ACCOUNT))
                    .primary()
                    .on_click(move |_, _, cx| {
                        add.update(cx, |state, cx| state.open_add_account(cx))
                    }),
            ),
        );
    }
    page = page.child(totals(state, handle, cx));
    page = page.children(unmanaged_warning(state, handle, cx));
    let busy = state.is_busy(Op::Switch) || state.is_busy(Op::Remove);
    page.children(
        rows.iter()
            .map(|row| account_card(row, busy, state, handle, cx)),
    )
}

fn totals(state: &AppState, handle: &Entity<AppState>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let summary = state.usage_summary();
    let refresh = handle.clone();
    let value = |known: bool, text: SharedString| {
        if known {
            text
        } else {
            tr(cx, keys::SUMMARY_UNKNOWN)
        }
    };
    let known = summary.known > 0;
    let missing = (summary.unknown > 0).then(|| usage::accounts_missing(summary.unknown, cx));
    let cell = |label: &str, text: SharedString| {
        captioned(tr(cx, label), figure(text, TextSize::Lg, cx))
            .flex_1()
            .min_w(theme.label_width())
            .gap_1()
    };
    let balance = value(known, format::cu(cx, summary.balance_cu));
    let rate = value(known, format::cu_rate(cx, summary.rate_cu_per_hour));
    let runtimes = value(known, summary.runtimes.to_string().into());
    tagged(
        format!("{TOTALS_SELECTOR} {balance}"),
        tagged(
            TOTALS_SELECTOR,
            Card::new()
                .header(
                    CardHeader::new(tr(cx, keys::ACCOUNTS_TOTAL_TITLE)).action(tagged(
                        REFRESH_SELECTOR,
                        IconButton::new("accounts-refresh", IconName::RefreshCw)
                            .tooltip(tr(cx, keys::USAGE_REFRESH))
                            .variant(ButtonVariant::Ghost)
                            .disabled(state.is_busy(Op::Usage))
                            .on_click(move |_, _, cx| {
                                refresh.update(cx, |state, cx| state.refresh_usage(true, cx))
                            }),
                    )),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_6()
                        .child(cell(keys::ACCOUNTS_BALANCE, balance.clone()))
                        .child(cell(keys::ACCOUNTS_RATE, rate))
                        .child(cell(keys::SUMMARY_RUNTIMES, runtimes)),
                )
                .children(missing.map(Caption::new))
                .children(state.usage_error().map(|error| {
                    tagged(
                        USAGE_ERROR_SELECTOR,
                        InlineMessage::new(Severity::Warning, explain(error, false, cx).message),
                    )
                })),
        ),
    )
    .w_full()
    .into_any_element()
}

fn account_card(
    row: &AccountRow,
    busy: bool,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let email = format::email(cx, row.account.email.as_deref());
    let mut actions = div().flex().items_center().gap_2();
    if !row.active {
        let (switch, id) = (handle.clone(), row.account.id.clone());
        actions = actions.child(tagged(
            switch_selector(&email),
            Button::new(
                SharedString::from(format!("switch-{}", row.account.id)),
                tr(cx, keys::ACCOUNTS_SWITCH),
            )
            .size(ControlSize::Sm)
            .disabled(busy)
            .on_click(move |_, _, cx| {
                switch.update(cx, |state, cx| state.request_switch(id.clone(), cx))
            }),
        ));
    }
    let block = row.remove_block.map(|block| match block {
        RemoveBlock::Active => tr(cx, keys::ACCOUNTS_REMOVE_ACTIVE),
        RemoveBlock::Running => tr(cx, keys::ACCOUNTS_REMOVE_RUNNING),
    });
    let (remove, id) = (handle.clone(), row.account.id.clone());
    let remove_button = Button::new(
        SharedString::from(format!("remove-{}", row.account.id)),
        tr(cx, keys::ACCOUNTS_REMOVE),
    )
    .icon(IconName::Trash2)
    .variant(ButtonVariant::Ghost)
    .size(ControlSize::Sm)
    .disabled(busy || block.is_some())
    .on_click(move |_, _, cx| remove.update(cx, |state, cx| state.request_remove(id.clone(), cx)));
    let remove_slot = div()
        .id(SharedString::from(format!(
            "remove-slot-{}",
            row.account.id
        )))
        .child(tagged(remove_selector(&email), remove_button));
    actions = actions.child(match &block {
        Some(reason) => remove_slot
            .tooltip(Tooltip::text(reason.clone()))
            .into_any_element(),
        None => remove_slot.into_any_element(),
    });
    let header = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(email.clone()))
                .children(
                    row.active
                        .then(|| Badge::new(tr(cx, keys::ACCOUNTS_ACTIVE)).tone(Tone::Success)),
                ),
        )
        .child(actions);
    let stats = usage_stats(row.usage.as_ref(), state, cx);
    tagged(
        row_selector(&email, row.active),
        Card::new()
            .child(header)
            .child(stats)
            .children(block.map(|reason| {
                div()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(theme.colors.fg_muted)
                    .child(reason)
            })),
    )
    .w_full()
    .into_any_element()
}

/// Balance, burn rate, time left and when it was updated; unknown and stale marked.
fn usage_stats(usage: Option<&AccountUsageView>, state: &AppState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let snapshot = usage.and_then(|usage| usage.snapshot);
    let unknown = || tr(cx, keys::SUMMARY_UNKNOWN);
    let cell = |label: &str, text: SharedString| {
        captioned(
            tr(cx, label),
            div().font_weight(FontWeight::MEDIUM).child(text),
        )
        .min_w(metrics::usage_figure_min_width(theme))
    };
    let left = usage::time_left(usage, cx);
    let updated = match (snapshot, state.usage_now()) {
        (Some(snapshot), Some(now)) => format::age(cx, snapshot.fetched_at, now),
        _ => unknown(),
    };
    let stale = snapshot.is_some_and(|s| usage_is_stale(&s, state.usage_now()));
    let failure = usage
        .and_then(|usage| usage.last_error.as_ref())
        .filter(|_| snapshot.is_none() || stale)
        .map(|error| {
            InlineMessage::new(
                Severity::Warning,
                format!(
                    "{} {}",
                    tr(cx, keys::ACCOUNTS_USAGE_FAILED),
                    explain(error, false, cx).message
                ),
            )
        });
    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_2()
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_6()
                .child(cell(
                    keys::ACCOUNTS_BALANCE,
                    snapshot.map_or_else(unknown, |s| format::cu(cx, s.balance_cu)),
                ))
                .child(cell(
                    keys::ACCOUNTS_RATE,
                    snapshot.map_or_else(unknown, |s| format::cu_rate(cx, s.rate_cu_per_hour)),
                ))
                .child(cell(keys::ACCOUNTS_TIME_LEFT, left))
                .child(cell(keys::ACCOUNTS_UPDATED, updated))
                .children(
                    stale.then(|| Badge::new(tr(cx, keys::USAGE_OUTDATED)).tone(Tone::Warning)),
                ),
        )
        .children(failure)
        .into_any_element()
}
