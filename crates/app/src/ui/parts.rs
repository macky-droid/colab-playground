//! Small pieces that more than one view draws the same way.

use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    motion::Spinner,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize, TextSize},
    typography::Caption,
};
use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, Hsla, IntoElement, ParentElement,
    SharedString, Styled, Window, div,
};

use crate::{
    i18n::{keys, tr},
    state::AppState,
    ui::tagged,
};

/// What is drawn over the window and closed by its own buttons.
#[derive(Clone, Copy)]
pub enum Layer {
    /// An account dialog (`ui::dialogs`).
    Dialog,
    /// Any other dialog (`ui::overlays`).
    Overlay,
}

impl Layer {
    fn cancel_selector(self) -> &'static str {
        match self {
            Layer::Dialog => "dialog-cancel",
            Layer::Overlay => "overlay-cancel",
        }
    }

    pub fn close(self, handle: &Entity<AppState>) -> impl Fn(&mut Window, &mut App) + 'static {
        let handle = handle.clone();
        move |_, cx| {
            handle.update(cx, |state, cx| match self {
                Layer::Dialog => state.close_dialog(cx),
                Layer::Overlay => state.close_overlay(cx),
            })
        }
    }

    pub fn cancel_button(self, label: SharedString, handle: Entity<AppState>) -> Div {
        self.cancel_button_when(label, handle, true)
    }

    pub fn cancel_button_when(
        self,
        label: SharedString,
        handle: Entity<AppState>,
        enabled: bool,
    ) -> Div {
        let close = self.close(&handle);
        tagged(
            self.cancel_selector(),
            Button::new(self.cancel_selector(), label)
                .variant(ButtonVariant::Ghost)
                .disabled(!enabled)
                .on_click(move |_, window, cx| close(window, cx)),
        )
    }
}

pub fn busy_line(id: impl Into<ElementId>, text: SharedString) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(Spinner::new(id).size(IconSize::Sm))
        .child(Caption::new(text))
}

pub fn field_label(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(cx.theme().text_size(TextSize::Sm))
        .font_weight(FontWeight::MEDIUM)
        .child(label.into())
}

pub fn field(label: impl Into<SharedString>, control: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(field_label(label, cx))
        .child(control)
}

/// Has no width and no gap: the views that share it differ in both.
pub fn captioned(label: SharedString, value: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .child(Caption::new(label))
        .child(value)
}

pub fn figure(value: SharedString, size: TextSize, cx: &App) -> Div {
    div()
        .text_size(cx.theme().text_size(size))
        .font_weight(FontWeight::SEMIBOLD)
        .child(value)
}

pub fn step_icon(icon: IconName, color: Hsla) -> AnyElement {
    Icon::new(icon)
        .size(IconSize::Sm)
        .color(color)
        .into_any_element()
}

pub fn step_spinner(id: impl Into<ElementId>) -> AnyElement {
    Spinner::new(id).size(IconSize::Sm).into_any_element()
}

pub fn cancel_work_button(
    selector: &'static str,
    cancelling: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let label = tr(
        cx,
        if cancelling {
            keys::NEW_CANCELLING
        } else {
            keys::ACTION_CANCEL
        },
    );
    tagged(
        selector,
        Button::new(selector, label)
            .variant(ButtonVariant::Danger)
            .loading(cancelling)
            .disabled(cancelling)
            .on_click(move |_, window, cx| on_click(window, cx)),
    )
}
