//! The app's own title strip, drawn in place of the OS title bar.
//!
//! One strip at Ely's title-bar height across the window top, on the same ground as the
//! sidebar and page, with no line under it. Its left part is as wide as the sidebar and carries
//! the sidebar's border up to the window top, with the app mark and name lined up with the nav
//! items; on macOS that part is left to the traffic lights, as in other Mac apps with a sidebar
//! (the menu bar names the app). Everything outside the window controls is an Ely
//! `drag_region`: on Windows it answers the system's hit test as caption, so dragging,
//! double-click to maximise and Aero Snap are native; Ely's `WindowControls` mark their buttons
//! as minimise / maximise / close areas, which gives the Windows 11 snap-layout flyout on the
//! maximise button. What each OS gets is decided by [`Chrome::layout`] in `platform`.

use ely_gpui_component::{
    primitives::{Icon, IconName},
    shell::{WindowControls, drag_region},
    theme::{ActiveTheme, IconSize, TextSize},
    typography::Ellipsis,
};
use gpui::{
    AnyElement, App, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, Window,
    div,
};

use crate::{
    i18n::{keys, tr},
    platform::Chrome,
};

pub const TITLE_BAR_SELECTOR: &str = "title-bar";
/// Selector of the strip's left part, as wide as the sidebar.
pub const BRAND_SELECTOR: &str = "title-bar-brand";
/// Selector of the app mark and name inside it, lined up with the nav items. Not drawn on
/// macOS.
pub const MARK_SELECTOR: &str = "title-bar-mark";
/// Selector around the window controls, when the app draws them. On Windows the buttons also
/// carry Ely's own selectors `caption-Minimize`, `caption-Maximize` and `caption-Close`.
pub const CONTROLS_SELECTOR: &str = "title-bar-controls";

pub fn render(chrome: Chrome, window: &mut Window, cx: &mut App) -> AnyElement {
    let layout = chrome.layout();
    let strip = if layout.draggable {
        drag_region(TITLE_BAR_SELECTOR, window, cx)
    } else {
        div().id(TITLE_BAR_SELECTOR)
    };
    let theme = cx.theme();
    let colors = &theme.colors;
    // Padded like the sidebar's nav items (column padding, then the item's border and padding)
    // so the mark sits over their icons.
    let mark = layout.mark.then(|| {
        div()
            .debug_selector(|| MARK_SELECTOR.into())
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .border_1()
            .px_2()
            .child(
                Icon::new(IconName::Zap)
                    .size(IconSize::Sm)
                    .color(colors.accent),
            )
            .child(
                div()
                    .min_w_0()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.fg)
                    .child(Ellipsis::new(tr(cx, keys::APP_NAME))),
            )
    });
    #[expect(
        clippy::disallowed_methods,
        reason = "Ely's vertical `Divider` takes its height as a share of its parent's and draws nothing in the strip"
    )]
    let brand = div()
        .debug_selector(|| BRAND_SELECTOR.into())
        .flex()
        .flex_none()
        .items_center()
        .h_full()
        .w(theme.sidebar_width(false))
        .border_r_1()
        .border_color(colors.border)
        .px_2()
        .children(mark);
    let controls = layout.controls.map(|platform| {
        div()
            .debug_selector(|| CONTROLS_SELECTOR.into())
            .flex_none()
            .h_full()
            .flex()
            .items_start()
            .child(WindowControls::new("window-controls").platform(platform))
    });
    // No ground of its own: it shows the root's (or, client-drawn on Linux, the rounded pane's
    // of Ely's `ResizeBorder`), so it cannot differ from the sidebar and page.
    strip
        .debug_selector(|| TITLE_BAR_SELECTOR.into())
        .flex()
        .flex_none()
        .w_full()
        .h(theme.titlebar_height())
        .child(brand)
        .child(div().flex_1().h_full())
        .children(controls)
        .into_any_element()
}
