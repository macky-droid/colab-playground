//! The sidebar: account switcher, navigation, and the usage and demo badges. The app mark
//! sits above it in the title strip (`titlebar`).

use colab_playground_core::backend::AccountUsageView;
use ely_gpui_component::{
    data_display::{Badge, Tone},
    layout::Sidebar,
    menus::{Menu, MenuItem, menu_under},
    navigation::NavItem,
    primitives::{FocusRing, Icon, IconName, Tooltip},
    theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize},
};
use gpui::{
    App, Entity, InteractiveElement, IntoElement, ParentElement, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};

use crate::{
    backend, format,
    i18n::{keys, tr, tr_args},
    route::Route,
    state::{AppState, SignInStatus, usage_is_stale, usage_stale_note},
    ui::tagged,
};

pub const ACCOUNT_SLOT_SELECTOR: &str = "sidebar-account";
/// Selector of the balance and burn-rate badge.
pub const USAGE_SLOT_SELECTOR: &str = "sidebar-usage";
/// Selector of the demo-data badge shown with the mock backend.
pub const DEMO_BADGE_SELECTOR: &str = "sidebar-demo";
/// Selector around the Settings entry while a newer release is available.
pub const UPDATE_MARK_SELECTOR: &str = "sidebar-update";

/// The sidebar for `state`; `handle` is the same state, for click handlers.
pub fn render(state: &AppState, handle: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let current = state.route();
    // A newer release stays marked on Settings (where About offers it) until this build is it.
    let update = state.update_available().is_some();
    let items = Route::ALL.map(|route| {
        let handle = handle.clone();
        let item = NavItem::new(
            route.nav_selector(),
            route.icon(),
            tr(cx, route.title_key()),
        )
        .active(route == current)
        .on_click(move |_, cx| handle.update(cx, |state, cx| state.navigate(route, cx)));
        let item = if update && route == Route::Settings {
            item.count(tr(cx, keys::UPDATE_BADGE))
        } else {
            item
        };
        let marked = if update && route == Route::Settings {
            UPDATE_MARK_SELECTOR
        } else {
            ""
        };
        tagged(route.nav_selector(), tagged(marked, item))
    });
    Sidebar::new("sidebar", false).child(
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_2()
            .child(account_slot(state, handle, cx))
            .child(div().flex().flex_col().gap_0p5().children(items))
            .child(div().flex_1())
            .child(badges(state, cx)),
    )
}

/// The account switcher: the active account with a menu to switch, add or manage; "Not
/// signed in" (leading to Accounts) otherwise.
fn account_slot(state: &AppState, handle: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let signed_in = matches!(state.sign_in_status(), SignInStatus::SignedIn { .. });
    let accounts = state.accounts().ready().filter(|_| signed_in);
    let Some(view) = accounts else {
        let handle = handle.clone();
        return div()
            .id(ACCOUNT_SLOT_SELECTOR)
            .tooltip(Tooltip::text(tr(cx, keys::SHELL_ACCOUNT_HINT)))
            .child(tagged(
                ACCOUNT_SLOT_SELECTOR,
                NavItem::new(
                    "account-slot",
                    IconName::UserPlus,
                    tr(cx, keys::SHELL_ACCOUNT_SIGNED_OUT),
                )
                .on_click(move |_, cx| {
                    handle.update(cx, |state, cx| state.navigate(Route::Accounts, cx))
                }),
            ))
            .into_any_element();
    };
    let no_email = tr(cx, keys::ACCOUNTS_NO_EMAIL);
    let label = state
        .active_email()
        .map_or(no_email.clone(), SharedString::from);
    let switches: Vec<MenuItem> = view
        .accounts
        .iter()
        .map(|account| {
            let active = view.active.as_ref() == Some(&account.id);
            let (id, handle) = (account.id.clone(), handle.clone());
            let email = account
                .email
                .clone()
                .map_or(no_email.clone(), SharedString::from);
            MenuItem::radio(email, active).on_click(move |_, cx| {
                if !active {
                    handle.update(cx, |state, cx| state.request_switch(id.clone(), cx));
                }
            })
        })
        .collect();
    let (add, manage) = (handle.clone(), handle.clone());
    let menu = Menu::new()
        .group(tr(cx, keys::SWITCHER_GROUP), switches)
        .separator()
        .item(
            MenuItem::new(tr(cx, keys::SWITCHER_ADD))
                .icon(IconName::UserPlus)
                .on_click(move |_, cx| add.update(cx, |state, cx| state.open_add_account(cx))),
        )
        .item(
            MenuItem::new(tr(cx, keys::SWITCHER_MANAGE))
                .icon(IconName::Users)
                .on_click(move |_, cx| {
                    manage.update(cx, |state, cx| state.navigate(Route::Accounts, cx))
                }),
        );
    AccountSwitcher { label, menu }.into_any_element()
}

/// The signed-in account slot. It is laid out like a `NavItem` (same height, padding, icon
/// size and hover) so its icon and label line up with the nav items under it, and opens
/// `menu` under itself. Ely's `DropdownMenu` is a centred button, so the trigger is ours and
/// the menu is hosted with `menu_under`.
#[derive(IntoElement)]
struct AccountSwitcher {
    label: SharedString,
    menu: Menu,
}

impl RenderOnce for AccountSwitcher {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (fg, muted, subtle, hover) =
            (colors.fg, colors.fg_muted, colors.fg_subtle, colors.hover);
        let trigger = div()
            .id("account-switcher-trigger")
            .debug_selector(|| ACCOUNT_SLOT_SELECTOR.into())
            .role(Role::Button)
            .aria_label(self.label.clone())
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h(theme.control_height(ControlSize::Md))
            .rounded(theme.radius(Radius::Md))
            .border_1()
            .border_color(gpui::transparent_black())
            .text_size(theme.text_size(TextSize::Sm))
            .text_color(muted)
            .cursor_pointer()
            .tab_index(0)
            .focus_ring(cx)
            .hover(move |style| style.bg(hover).text_color(fg))
            .child(Icon::new(IconName::User).size(IconSize::Sm).color(muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(self.label),
            )
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(IconSize::Sm)
                    .color(subtle),
            );
        menu_under(
            "account-switcher".into(),
            &self.menu,
            move |click| {
                trigger
                    .on_click(move |event, window, cx| click(event, window, cx))
                    .into_any_element()
            },
            window,
            cx,
        )
        .w_full()
    }
}

/// Balance · burn rate of the active account (stale marked, unknown never shown as zero) and
/// the demo-data badge.
fn badges(state: &AppState, cx: &App) -> impl IntoElement {
    let usage = state.active_usage();
    let (text, tone) = match usage.and_then(|usage| usage.snapshot) {
        Some(snapshot) => {
            let text = tr_args(
                cx,
                keys::USAGE_BADGE,
                &[
                    ("balance", &format::cu(cx, snapshot.balance_cu)),
                    ("rate", &format::cu_rate(cx, snapshot.rate_cu_per_hour)),
                ],
            );
            if usage_is_stale(&snapshot, state.usage_now()) {
                (
                    SharedString::from(format!("{text} · {}", tr(cx, keys::USAGE_OUTDATED))),
                    Tone::Warning,
                )
            } else {
                (text, Tone::Neutral)
            }
        }
        None => (tr(cx, keys::SHELL_USAGE), Tone::Neutral),
    };
    let tip = usage_tip(usage, state, cx);
    div()
        .flex()
        .flex_wrap()
        .gap_2()
        .px_2()
        .child(
            div()
                .id(USAGE_SLOT_SELECTOR)
                .tooltip(Tooltip::text(tip))
                .child(tagged(
                    format!("{USAGE_SLOT_SELECTOR} {text}"),
                    tagged(USAGE_SLOT_SELECTOR, Badge::new(text).tone(tone)),
                )),
        )
        .children(backend::shows_demo_badge(state.flavor()).then(|| {
            div()
                .id(DEMO_BADGE_SELECTOR)
                .tooltip(Tooltip::text(tr(cx, keys::SHELL_BACKEND_MOCK_HINT)))
                .child(tagged(
                    DEMO_BADGE_SELECTOR,
                    Badge::new(tr(cx, keys::SHELL_BACKEND_MOCK)).tone(Tone::Warning),
                ))
        }))
}

/// The badge's tooltip: time left, when it was updated, and whether the values are stale.
pub fn usage_tip(usage: Option<&AccountUsageView>, state: &AppState, cx: &App) -> SharedString {
    let Some(snapshot) = usage.and_then(|usage| usage.snapshot) else {
        return tr(cx, keys::USAGE_TIP_UNKNOWN);
    };
    let mut lines = vec![tr(cx, keys::SHELL_USAGE_HINT)];
    lines.push(match usage.and_then(|usage| usage.eta).or(snapshot.eta()) {
        None => tr(cx, keys::USAGE_TIP_IDLE),
        Some(eta) if eta.is_zero() => tr(cx, keys::USAGE_TIP_EMPTY),
        Some(eta) => tr_args(
            cx,
            keys::USAGE_TIP_LEFT,
            &[("time", &format::duration(cx, eta))],
        ),
    });
    if let Some(now) = state.usage_now() {
        lines.push(tr_args(
            cx,
            keys::USAGE_TIP_UPDATED,
            &[("age", &format::age(cx, snapshot.fetched_at, now))],
        ));
    }
    if usage_is_stale(&snapshot, state.usage_now()) {
        lines.push(tr(cx, usage_stale_note(&snapshot)));
    }
    let joined: Vec<&str> = lines.iter().map(SharedString::as_ref).collect();
    joined.join("\n").into()
}
