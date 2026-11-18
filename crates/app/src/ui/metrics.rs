//! Sizes the theme has no token for. They live together so they stay few, named, and easy to
//! check against the theme when it gains a token.
#![expect(
    clippy::disallowed_methods,
    reason = "the one module that turns numbers into sizes"
)]

use ely_gpui_component::theme::{ContainerSize, ControlSize, Theme};
use gpui::{Pixels, Rems, Size, px, rems, size};

pub const WINDOW_SIZE: Size<Pixels> = size(px(1180.0), px(780.0));
pub const WINDOW_MIN_SIZE: Size<Pixels> = size(px(880.0), px(560.0));

pub const RUN_LOG_HEIGHT: Rems = rems(14.0);

/// What a dialog keeps around its scrolling content: the window margin above and below, its
/// padding, title, action row and the gaps between them (about 209 px). Generous on purpose:
/// too much only leaves the dialog a little short, too little makes the dialog's own body
/// scroll as well.
pub const DIALOG_CHROME: Rems = rems(13.5);

pub const DIALOG_SCROLLED_MIN_HEIGHT: Rems = rems(8.0);

/// Ely's dialog card padding (`p_6`).
pub const DIALOG_PADDING: Rems = rems(1.5);

/// One part (hours, minutes or seconds) of a duration field; it follows the control height so
/// it keeps up with density.
pub fn duration_part_width(theme: &Theme) -> Rems {
    const CONTROL_HEIGHTS: f32 = 4.6;
    theme.control_height(ControlSize::Md) * CONTROL_HEIGHTS
}

pub fn path_field_width(theme: &Theme) -> Rems {
    const SMALL_CONTAINERS: f32 = 0.5;
    theme.container_width(ContainerSize::Sm) * SMALL_CONTAINERS
}

pub fn usage_figure_min_width(theme: &Theme) -> Rems {
    const LABEL_WIDTHS: f32 = 0.75;
    theme.label_width() * LABEL_WIDTHS
}
