//! A duration as hours, minutes (and optionally seconds) with units in the shown language.
//! Ely's `DurationPicker` labels its parts with fixed English `h` / `m` / `s`.

use std::{rc::Rc, time::Duration};

use ely_gpui_component::{forms::NumberInput, theme::ActiveTheme};
use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};

use crate::{
    i18n::{keys, tr},
    ui::metrics,
};

type OnChange = Rc<dyn Fn(Duration, &mut Window, &mut App)>;

const SECS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;
const SECS_PER_HOUR: u64 = SECS_PER_MINUTE * MINUTES_PER_HOUR;
const MAX_HOURS: f64 = 999.0;
const MAX_MINUTES: f64 = (MINUTES_PER_HOUR - 1) as f64;
const MAX_SECONDS: f64 = (SECS_PER_MINUTE - 1) as f64;

/// Hours and minutes (and seconds with [`DurationField::seconds`]).
#[derive(IntoElement)]
pub struct DurationField {
    id: SharedString,
    value: Duration,
    seconds: bool,
    on_change: OnChange,
}

impl DurationField {
    pub fn new(
        id: impl Into<SharedString>,
        value: Duration,
        on_change: impl Fn(Duration, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            value,
            seconds: false,
            on_change: Rc::new(on_change),
        }
    }

    pub fn seconds(mut self) -> Self {
        self.seconds = true;
        self
    }
}

impl RenderOnce for DurationField {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let total = self.value.as_secs();
        let (hours, minutes, seconds) = (
            total / SECS_PER_HOUR,
            total / SECS_PER_MINUTE % MINUTES_PER_HOUR,
            total % SECS_PER_MINUTE,
        );
        let compose = |h: u64, m: u64, s: u64| {
            Duration::from_secs(h * SECS_PER_HOUR + m * SECS_PER_MINUTE + s)
        };
        let width = metrics::duration_part_width(cx.theme());
        let part =
            |name: &str, value: u64, max: f64, unit: &str, set: Box<dyn Fn(u64) -> Duration>| {
                let on_change = self.on_change.clone();
                div().w(width).child(
                    NumberInput::new(
                        SharedString::from(format!("{}-{name}", self.id)),
                        value as f64,
                    )
                    .range(0.0, max)
                    .precision(0)
                    .step(1.0)
                    .suffix(div().child(tr(cx, unit)))
                    .on_change(move |v, window, cx| {
                        on_change(set(v.round().max(0.0) as u64), window, cx)
                    }),
                )
            };
        let mut row = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(part(
                "hours",
                hours,
                MAX_HOURS,
                keys::UNIT_HOURS,
                Box::new(move |h| compose(h, minutes, seconds)),
            ))
            .child(part(
                "minutes",
                minutes,
                MAX_MINUTES,
                keys::UNIT_MINUTES,
                Box::new(move |m| compose(hours, m, seconds)),
            ));
        if self.seconds {
            row = row.child(part(
                "seconds",
                seconds,
                MAX_SECONDS,
                keys::UNIT_SECONDS,
                Box::new(move |s| compose(hours, minutes, s)),
            ));
        }
        row
    }
}
