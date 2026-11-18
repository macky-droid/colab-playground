//! The options for a new runtime, shared by the New session dialog and the plugin run form:
//! hardware, the minimum GPU memory (with the A100 40/80 GB explanation), attempts, what to
//! do when they run out, and the cost limits. Values start from `Defaults`; edits apply to
//! this one action only. Each control's wrapper carries the selector `prov <field>`.

use std::{rc::Rc, time::Duration};

use colab_playground_core::{
    provision::{MAX_ATTEMPTS_CAP, OnExhausted, ProvisionSpec},
    sessions::Accelerator,
};
use ely_gpui_component::{
    forms::{Choice, NumberInput, Select, Switch},
    theme::{ActiveTheme, TextSize},
    typography::Caption,
};
use gpui::{AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, div};

use crate::{
    format,
    i18n::{keys, tr},
    ui::{
        duration_field::DurationField,
        pages::defaults::{
            MAX_CU_FIELD, MAX_VRAM_FIELD_GIB, MIB_PER_GIB, START_IDLE, START_MIN_BALANCE_CU,
            START_MIN_VRAM_GIB, START_SPEND_CU,
        },
        tagged,
    },
};

/// Where a runtime limit starts when turned on: core's default limit.
fn start_runtime() -> Duration {
    colab_playground_core::Defaults::default()
        .max_runtime
        .unwrap_or(START_IDLE)
}

pub type SpecEdit = Rc<dyn Fn(&mut App, &dyn Fn(&mut ProvisionSpec))>;

pub fn field_selector(field: &str) -> String {
    format!("prov {field}")
}

fn row(
    field: &str,
    label: SharedString,
    hint: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .py_1()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .flex_1()
                .min_w(theme.label_width())
                .child(
                    div()
                        .text_size(theme.text_size(TextSize::Sm))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .children(hint.map(Caption::new)),
        )
        .child(tagged(field_selector(field), control))
        .into_any_element()
}

/// A switch, and the value's control while it is on.
fn optional(
    field: &'static str,
    on: bool,
    edit: &SpecEdit,
    set: fn(&mut ProvisionSpec, bool),
    value: AnyElement,
    cx: &App,
) -> AnyElement {
    let toggle = edit.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            Switch::new(SharedString::from(format!("prov-switch-{field}")), on)
                .name(tr(cx, keys::DEF_LIMIT))
                .on_change(move |on, _, cx| toggle(cx, &|spec| set(spec, on))),
        )
        .child(if on {
            value
        } else {
            Caption::new(tr(cx, keys::DEF_NO_LIMIT)).into_any_element()
        })
        .into_any_element()
}

fn cu_field(
    id: &'static str,
    value: f64,
    edit: &SpecEdit,
    set: fn(&mut ProvisionSpec, f64),
    cx: &App,
) -> AnyElement {
    let edit = edit.clone();
    div()
        .w(cx.theme().menu_width())
        .child(
            NumberInput::new(id, value)
                .range(0.0, MAX_CU_FIELD)
                .precision(1)
                .step(1.0)
                .suffix(div().child(tr(cx, keys::UNIT_CU)))
                .on_change(move |v, _, cx| edit(cx, &|spec| set(spec, v))),
        )
        .into_any_element()
}

fn duration_field(
    id: &'static str,
    value: Duration,
    edit: &SpecEdit,
    set: fn(&mut ProvisionSpec, Duration),
) -> AnyElement {
    let edit = edit.clone();
    DurationField::new(id, value, move |v, _, cx| edit(cx, &|spec| set(spec, v))).into_any_element()
}

/// The options for `spec`; `prefix` keeps element ids apart when two forms exist.
pub fn render(prefix: &'static str, spec: &ProvisionSpec, edit: SpecEdit, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let id = |name: &str| SharedString::from(format!("{prefix}-{name}"));
    let accelerator = {
        let edit = edit.clone();
        div().w(theme.menu_width()).child(
            Select::new(
                id("accelerator"),
                Accelerator::ALL
                    .map(|a| Choice::new(a.as_str(), tr(cx, format::accelerator_key(a)))),
            )
            .label(tr(cx, keys::DEF_ACCELERATOR))
            .selected(spec.accelerator.as_str())
            .on_change(move |value, _, cx| {
                if let Some(a) = Accelerator::ALL
                    .into_iter()
                    .find(|a| a.as_str() == value.as_ref())
                {
                    edit(cx, &|spec| spec.accelerator = a);
                }
            }),
        )
    };
    let high_mem = {
        let edit = edit.clone();
        Switch::new(id("high-mem"), spec.high_mem)
            .name(tr(cx, keys::DEF_HIGH_MEM))
            .on_change(move |on, _, cx| edit(cx, &|spec| spec.high_mem = on))
    };
    let gpu = spec.accelerator.is_gpu();
    let min_vram = optional(
        "min_vram",
        spec.min_vram_mib.is_some(),
        &edit,
        |spec, on| spec.min_vram_mib = on.then_some(START_MIN_VRAM_GIB * MIB_PER_GIB),
        {
            let edit = edit.clone();
            div()
                .w(theme.menu_width())
                .child(
                    NumberInput::new(
                        id("min-vram"),
                        f64::from(
                            spec.min_vram_mib
                                .unwrap_or(START_MIN_VRAM_GIB * MIB_PER_GIB)
                                / MIB_PER_GIB,
                        ),
                    )
                    .range(1.0, MAX_VRAM_FIELD_GIB)
                    .precision(0)
                    .step(1.0)
                    .suffix(div().child(tr(cx, keys::UNIT_GB)))
                    .on_change(move |v, _, cx| {
                        edit(cx, &|spec| {
                            spec.min_vram_mib = Some((v.round() as u32).max(1) * MIB_PER_GIB)
                        })
                    }),
                )
                .into_any_element()
        },
        cx,
    );
    let attempts = {
        let edit = edit.clone();
        div().w(theme.menu_width()).child(
            NumberInput::new(id("attempts"), f64::from(spec.attempts()))
                .range(1.0, f64::from(MAX_ATTEMPTS_CAP))
                .precision(0)
                .step(1.0)
                .on_change(move |v, _, cx| edit(cx, &|spec| spec.max_attempts = v.round() as u32)),
        )
    };
    let exhausted = {
        let edit = edit.clone();
        div().w(theme.menu_width()).child(
            Select::new(
                id("on-exhausted"),
                [
                    Choice::new("stop", tr(cx, keys::DEF_EXHAUSTED_STOP)),
                    Choice::new("keep", tr(cx, keys::DEF_EXHAUSTED_KEEP)),
                ],
            )
            .label(tr(cx, keys::DEF_ON_EXHAUSTED))
            .selected(match spec.on_exhausted {
                OnExhausted::StopAndFail => "stop",
                OnExhausted::KeepLast => "keep",
            })
            .on_change(move |value, _, cx| {
                let keep = value.as_ref() == "keep";
                edit(cx, &|spec| {
                    spec.on_exhausted = if keep {
                        OnExhausted::KeepLast
                    } else {
                        OnExhausted::StopAndFail
                    }
                });
            }),
        )
    };
    let guard = &spec.guard;
    let max_runtime = optional(
        "max_runtime",
        guard.max_runtime.is_some(),
        &edit,
        |spec, on| spec.guard.max_runtime = on.then(start_runtime),
        duration_field(
            "prov-max-runtime",
            guard.max_runtime.unwrap_or_else(start_runtime),
            &edit,
            |spec, v| spec.guard.max_runtime = Some(v),
        ),
        cx,
    );
    let max_spend = optional(
        "max_spend",
        guard.max_spend_cu.is_some(),
        &edit,
        |spec, on| spec.guard.max_spend_cu = on.then_some(START_SPEND_CU),
        cu_field(
            "prov-max-spend",
            guard.max_spend_cu.unwrap_or(START_SPEND_CU),
            &edit,
            |spec, v| spec.guard.max_spend_cu = Some(v),
            cx,
        ),
        cx,
    );
    let min_balance = optional(
        "min_balance",
        guard.min_balance_cu.is_some(),
        &edit,
        |spec, on| spec.guard.min_balance_cu = on.then_some(START_MIN_BALANCE_CU),
        cu_field(
            "prov-min-balance",
            guard.min_balance_cu.unwrap_or(START_MIN_BALANCE_CU),
            &edit,
            |spec, v| spec.guard.min_balance_cu = Some(v),
            cx,
        ),
        cx,
    );
    let idle = optional(
        "idle_timeout",
        guard.idle_timeout.is_some(),
        &edit,
        |spec, on| spec.guard.idle_timeout = on.then_some(START_IDLE),
        duration_field(
            "prov-idle",
            guard.idle_timeout.unwrap_or(START_IDLE),
            &edit,
            |spec, v| spec.guard.idle_timeout = Some(v),
        ),
        cx,
    );
    let mut rows = vec![
        row(
            "accelerator",
            tr(cx, keys::DEF_ACCELERATOR),
            None,
            accelerator,
            cx,
        ),
        row("high_mem", tr(cx, keys::DEF_HIGH_MEM), None, high_mem, cx),
    ];
    if gpu {
        rows.push(row(
            "min_vram",
            tr(cx, keys::DEF_MIN_VRAM),
            Some(tr(cx, keys::PROV_VRAM_LOTTERY)),
            min_vram,
            cx,
        ));
    }
    rows.extend([
        row(
            "attempts",
            tr(cx, keys::DEF_ATTEMPTS),
            Some(tr(cx, keys::PROV_ATTEMPTS_BILLED)),
            attempts,
            cx,
        ),
        row(
            "on_exhausted",
            tr(cx, keys::DEF_ON_EXHAUSTED),
            None,
            exhausted,
            cx,
        ),
        row(
            "max_runtime",
            tr(cx, keys::DEF_MAX_RUNTIME),
            Some(tr(cx, keys::DEF_MAX_RUNTIME_HINT)),
            max_runtime,
            cx,
        ),
        row(
            "max_spend",
            tr(cx, keys::DEF_MAX_SPEND),
            None,
            max_spend,
            cx,
        ),
        row(
            "min_balance",
            tr(cx, keys::DEF_MIN_BALANCE),
            None,
            min_balance,
            cx,
        ),
        row("idle_timeout", tr(cx, keys::DEF_IDLE), None, idle, cx),
    ]);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(rows)
        .into_any_element()
}
