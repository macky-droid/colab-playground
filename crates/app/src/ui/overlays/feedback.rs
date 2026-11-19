//! Asking for a star or feedback (when it comes: `state::feedback`). The two choices sit in
//! the body, side by side, which leaves their labels room in every language; the row of
//! actions keeps only the way out.

use ely_gpui_component::{
    buttons::Button,
    overlays::Dialog,
    primitives::IconName,
    typography::{Caption, Paragraph},
};
use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, Styled, div};

use super::{cancel_button, close};
use crate::{
    i18n::{keys, tr},
    state::AppState,
    ui::tagged,
};

/// Selector of the prompt's text.
pub const FEEDBACK_PROMPT: &str = "feedback-prompt";
pub const FEEDBACK_STAR: &str = "feedback-star";
pub const FEEDBACK_GIVE: &str = "feedback-give";

pub(super) fn render(state: &AppState, handle: &Entity<AppState>, cx: &App) -> AnyElement {
    let dismiss = tr(
        cx,
        if state.feedback_asked_for_the_last_time() {
            keys::FEEDBACK_DONT_ASK_AGAIN
        } else {
            keys::FEEDBACK_NOT_NOW
        },
    );
    let (star, give) = (handle.clone(), handle.clone());
    let choices = div()
        .flex()
        .gap_2()
        .child(
            tagged(
                FEEDBACK_STAR,
                Button::new(FEEDBACK_STAR, tr(cx, keys::FEEDBACK_STAR))
                    .primary()
                    .icon(IconName::Star)
                    .full_width()
                    .on_click(move |_, _, cx| {
                        star.update(cx, |state, cx| state.star_from_prompt(cx))
                    }),
            )
            .flex_1()
            .min_w_0(),
        )
        .child(
            tagged(
                FEEDBACK_GIVE,
                Button::new(FEEDBACK_GIVE, tr(cx, keys::FEEDBACK_GIVE))
                    .icon(IconName::MessageSquare)
                    .full_width()
                    .on_click(move |_, _, cx| {
                        give.update(cx, |state, cx| state.give_feedback_from_prompt(cx))
                    }),
            )
            .flex_1()
            .min_w_0(),
        );
    Dialog::new("feedback", tr(cx, keys::FEEDBACK_TITLE), close(handle))
        .child(tagged(
            FEEDBACK_PROMPT,
            Paragraph::new(tr(cx, keys::FEEDBACK_BODY)),
        ))
        .child(choices)
        .child(Caption::new(tr(cx, keys::FEEDBACK_ALSO_IN_ABOUT)))
        .action({
            let handle = handle.clone();
            move |_| cancel_button(dismiss, handle)
        })
        .into_any_element()
}
