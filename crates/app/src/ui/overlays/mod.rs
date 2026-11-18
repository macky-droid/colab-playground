//! Dialogs other than the account ones: stop and end billing, new session (with its live
//! progress), forward a port, install or remove a plugin, and quitting while runtimes bill.
//! Every button carries a selector (`overlay-cancel` for the dismissing one).

mod forward;
mod new_session;
mod plugin;
mod quit;
mod stop;

use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    motion::Spinner,
    theme::IconSize,
    typography::Caption,
};
use gpui::{
    AnyElement, App, Entity, InteractiveElement, IntoElement, ParentElement, Rems, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};

use crate::{
    state::{AppState, Overlay},
    ui::{Inputs, tagged},
};

pub use forward::{FORWARD_SUBMIT, RESERVED_NOTE};
pub use new_session::{NEW_SESSION_CANCEL_RUN, NEW_SESSION_LIMIT, NEW_SESSION_START};
pub use plugin::{INSTALL_CONFIRM, REMOVE_CONFIRM};
pub use quit::{QUIT_CHECKING, QUIT_LEAVE, QUIT_STOP_ALL, QUIT_UNCHECKED};
pub use stop::{CANCEL_AUTO_STOP_CONFIRM, STOP_CONFIRM};

/// The open overlay, if any (an account dialog takes precedence).
pub fn render(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> Option<AnyElement> {
    if state.dialog().is_some() {
        return None;
    }
    Some(match state.overlay()? {
        Overlay::Stop(prompt) => stop::render(prompt, state, handle, cx),
        Overlay::CancelAutoStop(prompt) => stop::cancel_auto_stop(prompt, state, handle, cx),
        Overlay::NewSession => new_session::render(state, handle, inputs, cx)?,
        Overlay::Forward(form) => forward::render(form, state, handle, inputs, cx),
        Overlay::InstallPlugin(prompt) => plugin::install(prompt, state, handle, cx),
        Overlay::RemovePlugin(prompt) => plugin::remove(prompt, state, handle, cx),
        Overlay::Quit(prompt) => quit::render(prompt, state, handle, cx),
    })
}

fn close(handle: &Entity<AppState>) -> impl Fn(&mut Window, &mut App) + 'static {
    let handle = handle.clone();
    move |_, cx| handle.update(cx, |state, cx| state.close_overlay(cx))
}

fn cancel_button(label: SharedString, handle: Entity<AppState>) -> impl IntoElement {
    cancel_button_when(label, handle, true)
}

/// The dismissing button, disabled while `enabled` is false.
fn cancel_button_when(
    label: SharedString,
    handle: Entity<AppState>,
    enabled: bool,
) -> impl IntoElement {
    tagged(
        "overlay-cancel",
        Button::new("overlay-cancel", label)
            .variant(ButtonVariant::Ghost)
            .disabled(!enabled)
            .on_click(move |_, _, cx| handle.update(cx, |state, cx| state.close_overlay(cx))),
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

/// Tall content scrolls inside the dialog, past `max_height`.
#[expect(
    clippy::disallowed_methods,
    reason = "under a maximum height, Ely's `ScrollArea` leaves the Forward dialog's body cut off and unscrollable"
)]
fn scrolled(id: &'static str, max_height: Rems, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(id)
        .max_h(max_height)
        .overflow_y_scroll()
        .pr_2()
        .child(content)
}
