//! One function per page. A page that needs local state (text fields, scroll positions) keeps it
//! in `AppState` or in an entity the root view owns (see `ui::Inputs`).

pub mod accounts;
pub mod defaults;
pub mod overview;
pub mod plugins;
pub mod sessions;
pub mod settings;

use ely_gpui_component::{
    layout::Page,
    theme::{ActiveTheme, ContainerSize},
};
use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, Styled, div};

use crate::{
    i18n::tr,
    route::Route,
    state::AppState,
    ui::{Inputs, tagged},
};

pub fn render(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> AnyElement {
    let route = state.route();
    let notice = update_banner(state, handle, cx);
    let page = match route {
        Route::Overview => overview::render(state, handle, cx),
        Route::Sessions => sessions::render(state, handle, cx),
        Route::Plugins => plugins::render(state, handle, inputs, cx),
        Route::Accounts => accounts::render(state, handle, cx),
        Route::Settings => settings::render(state, handle, inputs, cx),
    };
    let framed = frame(route, page, cx);
    match notice {
        Some(notice) => div()
            .w_full()
            .flex()
            .flex_col()
            .child(notice)
            .child(framed)
            .into_any_element(),
        None => framed,
    }
}

pub const UPDATE_NOTICE_SELECTOR: &str = "update-notice";

/// A newer release, announced once per version above every page until dismissed for that
/// version; it offers the release page (the app downloads nothing itself).
fn update_banner(state: &AppState, handle: &Entity<AppState>, cx: &App) -> Option<AnyElement> {
    let (version, _) = state.update_notice()?;
    let theme = cx.theme();
    let (open, dismiss) = (handle.clone(), handle.clone());
    let dismissed = version.clone();
    let actions = div()
        .flex()
        .gap_2()
        .child(tagged(
            "update-notice-open",
            ely_gpui_component::buttons::Button::new(
                "update-notice-open",
                tr(cx, crate::i18n::keys::UPDATE_OPEN_PAGE),
            )
            .size(ely_gpui_component::theme::ControlSize::Sm)
            .primary()
            .on_click(move |_, _, cx| open.update(cx, |state, cx| state.open_update_page(cx))),
        ))
        .child(tagged(
            "update-notice-dismiss",
            ely_gpui_component::buttons::Button::new(
                "update-notice-dismiss",
                tr(cx, crate::i18n::keys::UPDATE_DISMISS),
            )
            .size(ely_gpui_component::theme::ControlSize::Sm)
            .variant(ely_gpui_component::buttons::ButtonVariant::Ghost)
            .on_click(move |_, _, cx| {
                dismiss.update(cx, |state, cx| state.dismiss_update(dismissed.clone(), cx))
            }),
        ));
    Some(
        tagged(
            UPDATE_NOTICE_SELECTOR,
            div()
                .w_full()
                .max_w(theme.container_width(ContainerSize::Lg))
                .mx_auto()
                .px_6()
                .pt_4()
                .child(
                    ely_gpui_component::feedback::Alert::new(
                        "update-notice",
                        ely_gpui_component::primitives::Severity::Info,
                        crate::i18n::tr_args(
                            cx,
                            crate::i18n::keys::UPDATE_NOTICE_TITLE,
                            &[("version", &version)],
                        ),
                    )
                    .body(tr(cx, crate::i18n::keys::UPDATE_NOTICE_BODY))
                    .action(actions),
                ),
        )
        .w_full()
        .into_any_element(),
    )
}

/// The page under its title, centred at a readable width. In test builds the frame carries the
/// selector `page <slug> <title>`, so tests can check which page shows in which language.
fn frame(route: Route, page: Page, cx: &App) -> AnyElement {
    let title = tr(cx, route.title_key());
    let theme = cx.theme();
    tagged(
        format!("page {} {title}", route.slug()),
        div()
            .w_full()
            .max_w(theme.container_width(ContainerSize::Lg))
            .mx_auto()
            .child(page),
    )
    .w_full()
    .into_any_element()
}

pub fn page(route: Route, subtitle_key: &str, cx: &App) -> Page {
    Page::new(tr(cx, route.title_key())).subtitle(tr(cx, subtitle_key))
}
