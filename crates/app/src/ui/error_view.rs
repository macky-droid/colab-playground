//! Showing an error: what happened, what to do, and a button when the app can act.

use std::rc::Rc;

use colab_playground_core::Error;
use ely_gpui_component::{
    buttons::Button, feedback::Alert, primitives::Severity, theme::ControlSize,
};
use gpui::{AnyElement, App, Entity, IntoElement, SharedString, Window};

use crate::{
    errors::{ErrorAction, explain},
    i18n::tr,
    platform,
    route::Route,
    state::AppState,
    ui::tagged,
};

pub type Retry = Rc<dyn Fn(&mut Window, &mut App)>;

pub fn perform(
    action: ErrorAction,
    handle: &Entity<AppState>,
    retry: Option<&Retry>,
    window: &mut Window,
    cx: &mut App,
) {
    match action {
        ErrorAction::Retry => {
            if let Some(retry) = retry {
                retry(window, cx);
            }
        }
        ErrorAction::Refresh => handle.update(cx, |state, cx| state.check_environment(cx)),
        ErrorAction::InstallCli => {
            handle.update(cx, |state, cx| state.navigate(Route::Overview, cx))
        }
        ErrorAction::AddAccount | ErrorAction::SignInAgain => handle.update(cx, |state, cx| {
            state.navigate(Route::Accounts, cx);
            state.open_add_account(cx);
        }),
        ErrorAction::SwitchAccount => {
            handle.update(cx, |state, cx| state.navigate(Route::Accounts, cx))
        }
        ErrorAction::InstallWsl => cx.open_url(platform::WSL_INSTALL_URL),
    }
}

/// `error` as an Ely alert: the message, the hint and detail as its body, and the next step's
/// button (selector `error-action <id>`). `title` replaces the message as the heading.
pub fn alert(
    id: &str,
    title: Option<SharedString>,
    error: &Error,
    retry: Option<Retry>,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let explained = explain(error, retry.is_some(), cx);
    let mut body: Vec<SharedString> = Vec::new();
    if title.is_some() {
        body.push(explained.message.clone());
    }
    body.extend(explained.hint.clone());
    body.extend(explained.detail.clone());
    let heading = title.unwrap_or_else(|| explained.message.clone());
    let mut alert = Alert::new(
        SharedString::from(format!("error-{id}")),
        Severity::Danger,
        heading,
    );
    if !body.is_empty() {
        let joined: Vec<&str> = body.iter().map(SharedString::as_ref).collect();
        alert = alert.body(joined.join("\n"));
    }
    if let Some(action) = explained.action {
        let handle = handle.clone();
        let button = Button::new(
            SharedString::from(format!("error-action-{id}")),
            tr(cx, action.label_key()),
        )
        .size(ControlSize::Sm)
        .on_click(move |_, window, cx| perform(action, &handle, retry.as_ref(), window, cx));
        alert = alert.action(tagged(format!("error-action {id}"), button));
    }
    tagged(format!("error-{id}"), alert).into_any_element()
}
