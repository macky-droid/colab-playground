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
    layout::ScrollArea,
    motion::Spinner,
    theme::{ActiveTheme, IconSize},
    typography::Caption,
};
use gpui::{
    AnyElement, App, Entity, IntoElement, ParentElement, Rems, RenderOnce, ScrollHandle,
    SharedString, Styled, Window, div,
};

use crate::{
    state::{AppState, Overlay},
    ui::{Inputs, tagged},
};

pub use forward::{FORWARD_BODY, FORWARD_SUBMIT, RESERVED_NOTE};
pub use new_session::{
    NEW_SESSION_CANCEL_RUN, NEW_SESSION_FORM, NEW_SESSION_LIMIT, NEW_SESSION_START,
};
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

/// What a dialog keeps around its scrolling content: the window margin above and below, its
/// padding, title, action row and the gaps between them (about 209 px). Generous on purpose:
/// too much only leaves the dialog a little short, too little makes the dialog's own body
/// scroll as well.
const DIALOG_CHROME: Rems = Rems(13.5);

/// The shortest the scrolling content gets, however small the window.
const MIN_SCROLLED: Rems = Rems(8.0);

/// A dialog's whole content (the dialog has no description, or `DIALOG_CHROME` is short), as
/// tall as the window allows and scrolling beyond that, with the page's overlay scrollbar and
/// soft edges where more content continues. A `handle` from the dialog's state lets it bring a
/// row into view; without one, each opening starts at the top as well.
#[derive(IntoElement)]
struct Scrolled {
    id: &'static str,
    handle: Option<ScrollHandle>,
    content: AnyElement,
}

fn scrolled(
    id: &'static str,
    handle: Option<&ScrollHandle>,
    content: impl IntoElement,
) -> Scrolled {
    Scrolled {
        id,
        handle: handle.cloned(),
        content: content.into_any_element(),
    }
}

impl RenderOnce for Scrolled {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rem = window.rem_size();
        let room = (window.viewport_size().height - DIALOG_CHROME.to_pixels(rem))
            .max(MIN_SCROLLED.to_pixels(rem));
        // The bar's track takes presses at the right edge, so the content keeps clear of it.
        let gutter = cx.theme().scrollbar_thickness().to_pixels(rem) * 2.0;
        let area = ScrollArea::new(self.id).vertical().shadows();
        let area = match &self.handle {
            Some(handle) => area.track_scroll(handle),
            None => area,
        };
        tagged(
            self.id,
            area
                // A column, so the scrolling body shrinks to the room instead of clipping.
                .flex()
                .flex_col()
                .max_h(room)
                .child(div().pr(gutter).child(self.content)),
        )
    }
}
