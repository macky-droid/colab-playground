//! The options of a new runtime that Settings (as `Defaults`) and the provision form (as a
//! `ProvisionSpec`) both offer. One set of controls serves both: the caller says where a value
//! is read and written, and lays the control out in its own row.
//!
//! Optional limits have a switch: off means no limit (`None`). Turning one on starts the field
//! at a `START_*` value below; that is only where the field starts, the user picks the value.

use std::{rc::Rc, time::Duration};

use colab_playground_core::{
    Defaults,
    provision::{MAX_ATTEMPTS_CAP, OnExhausted},
    sessions::Accelerator,
};
use ely_gpui_component::{
    forms::{Choice, NumberInput, Select, Switch},
    theme::ActiveTheme,
    typography::Caption,
};
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window, div};

use crate::{
    format,
    i18n::{keys, tr},
    ui::{duration_field::DurationField, tagged},
};

/// Where an idle limit starts when turned on.
pub const START_IDLE: Duration = Duration::from_secs(30 * 60);
/// Where a spending cap starts when turned on, in compute units.
pub const START_SPEND_CU: f64 = 10.0;
/// Where a balance floor starts when turned on, in compute units.
pub const START_MIN_BALANCE_CU: f64 = 5.0;
/// Where a minimum GPU memory starts when turned on, in GiB.
pub const START_MIN_VRAM_GIB: u32 = 40;
/// The largest compute-unit amount the fields accept.
pub const MAX_CU_FIELD: f64 = 100_000.0;
/// The largest GPU memory the field accepts, in GiB.
pub const MAX_VRAM_FIELD_GIB: f64 = 1024.0;
/// GPU memory is set in GiB and stored in MiB.
pub const MIB_PER_GIB: u32 = 1024;

const EXHAUSTED_STOP: &str = "stop";
const EXHAUSTED_KEEP: &str = "keep";

/// Where a runtime limit starts when turned on: core's default limit.
fn start_runtime() -> Duration {
    Defaults::default().max_runtime.unwrap_or(START_IDLE)
}

type Set<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub struct Controls<'a> {
    /// Starts every element id, which keeps the ids of two forms apart.
    pub prefix: &'static str,
    /// The selector around an optional limit's switch, given its field, where tests need one.
    pub switch_selector: Option<fn(&str) -> String>,
    pub cx: &'a App,
}

impl Controls<'_> {
    fn id(&self, name: &str) -> SharedString {
        SharedString::from(format!("{}-{name}", self.prefix))
    }

    /// A switch, and the value's control while it is on.
    fn optional<T: Copy + 'static>(
        &self,
        field: &str,
        on: bool,
        start: T,
        set: Set<Option<T>>,
        value: AnyElement,
    ) -> AnyElement {
        let switch = Switch::new(self.id(&format!("switch-{field}")), on)
            .name(tr(self.cx, keys::DEF_LIMIT))
            .on_change(move |on, window, cx| set(on.then_some(start), window, cx));
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(match self.switch_selector {
                Some(selector) => tagged(selector(field), switch).into_any_element(),
                None => switch.into_any_element(),
            })
            .child(if on {
                value
            } else {
                Caption::new(tr(self.cx, keys::DEF_NO_LIMIT)).into_any_element()
            })
            .into_any_element()
    }

    fn duration_limit(
        &self,
        field: &str,
        name: &str,
        value: Option<Duration>,
        start: Duration,
        set: impl Fn(Option<Duration>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let set: Set<Option<Duration>> = Rc::new(set);
        let input = DurationField::new(self.id(name), value.unwrap_or(start), {
            let set = set.clone();
            move |v, window, cx| set(Some(v), window, cx)
        })
        .into_any_element();
        self.optional(field, value.is_some(), start, set, input)
    }

    fn cu_limit(
        &self,
        field: &str,
        name: &str,
        value: Option<f64>,
        start: f64,
        set: impl Fn(Option<f64>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let set: Set<Option<f64>> = Rc::new(set);
        let input = div()
            .w(self.cx.theme().menu_width())
            .child(
                NumberInput::new(self.id(name), value.unwrap_or(start))
                    .range(0.0, MAX_CU_FIELD)
                    .precision(1)
                    .step(1.0)
                    .suffix(div().child(tr(self.cx, keys::UNIT_CU)))
                    .on_change({
                        let set = set.clone();
                        move |v, window, cx| set(Some(v), window, cx)
                    }),
            )
            .into_any_element();
        self.optional(field, value.is_some(), start, set, input)
    }

    pub fn max_runtime(
        &self,
        value: Option<Duration>,
        set: impl Fn(Option<Duration>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        self.duration_limit("max_runtime", "max-runtime", value, start_runtime(), set)
    }

    pub fn max_spend(
        &self,
        value: Option<f64>,
        set: impl Fn(Option<f64>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        self.cu_limit("max_spend", "max-spend", value, START_SPEND_CU, set)
    }

    pub fn min_balance(
        &self,
        value: Option<f64>,
        set: impl Fn(Option<f64>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        self.cu_limit(
            "min_balance",
            "min-balance",
            value,
            START_MIN_BALANCE_CU,
            set,
        )
    }

    pub fn idle_timeout(
        &self,
        value: Option<Duration>,
        set: impl Fn(Option<Duration>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        self.duration_limit("idle_timeout", "idle", value, START_IDLE, set)
    }

    /// The minimum GPU memory, shown in GiB and kept in MiB.
    pub fn min_vram(
        &self,
        mib: Option<u32>,
        set: impl Fn(Option<u32>, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let start = START_MIN_VRAM_GIB * MIB_PER_GIB;
        let set: Set<Option<u32>> = Rc::new(set);
        let input = div()
            .w(self.cx.theme().menu_width())
            .child(
                NumberInput::new(
                    self.id("min-vram"),
                    f64::from(mib.unwrap_or(start) / MIB_PER_GIB),
                )
                .range(1.0, MAX_VRAM_FIELD_GIB)
                .precision(0)
                .step(1.0)
                .suffix(div().child(tr(self.cx, keys::UNIT_GB)))
                .on_change({
                    let set = set.clone();
                    move |v, window, cx| {
                        set(Some((v.round() as u32).max(1) * MIB_PER_GIB), window, cx)
                    }
                }),
            )
            .into_any_element();
        self.optional("min_vram", mib.is_some(), start, set, input)
    }

    pub fn accelerator(
        &self,
        selected: Accelerator,
        set: impl Fn(Accelerator, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .w(self.cx.theme().menu_width())
            .child(
                Select::new(
                    self.id("accelerator"),
                    Accelerator::ALL
                        .map(|a| Choice::new(a.as_str(), tr(self.cx, format::accelerator_key(a)))),
                )
                .label(tr(self.cx, keys::DEF_ACCELERATOR))
                .selected(selected.as_str())
                .on_change(move |value, window, cx| {
                    if let Some(a) = Accelerator::ALL
                        .into_iter()
                        .find(|a| a.as_str() == value.as_ref())
                    {
                        set(a, window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    pub fn high_mem(
        &self,
        on: bool,
        set: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        Switch::new(self.id("high-mem"), on)
            .name(tr(self.cx, keys::DEF_HIGH_MEM))
            .on_change(set)
            .into_any_element()
    }

    pub fn attempts(
        &self,
        value: u32,
        set: impl Fn(u32, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .w(self.cx.theme().menu_width())
            .child(
                NumberInput::new(self.id("attempts"), f64::from(value))
                    .range(1.0, f64::from(MAX_ATTEMPTS_CAP))
                    .precision(0)
                    .step(1.0)
                    .on_change(move |v, window, cx| set(v.round() as u32, window, cx)),
            )
            .into_any_element()
    }

    pub fn on_exhausted(
        &self,
        selected: OnExhausted,
        set: impl Fn(OnExhausted, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .w(self.cx.theme().menu_width())
            .child(
                Select::new(
                    self.id("on-exhausted"),
                    [
                        Choice::new(EXHAUSTED_STOP, tr(self.cx, keys::DEF_EXHAUSTED_STOP)),
                        Choice::new(EXHAUSTED_KEEP, tr(self.cx, keys::DEF_EXHAUSTED_KEEP)),
                    ],
                )
                .label(tr(self.cx, keys::DEF_ON_EXHAUSTED))
                .selected(match selected {
                    OnExhausted::StopAndFail => EXHAUSTED_STOP,
                    OnExhausted::KeepLast => EXHAUSTED_KEEP,
                })
                .on_change(move |value, window, cx| {
                    let choice = if value.as_ref() == EXHAUSTED_KEEP {
                        OnExhausted::KeepLast
                    } else {
                        OnExhausted::StopAndFail
                    };
                    set(choice, window, cx);
                }),
            )
            .into_any_element()
    }
}
