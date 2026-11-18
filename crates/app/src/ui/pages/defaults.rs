//! Settings sections for core's `Defaults` and for where the CLI lives.
//!
//! Every field of `Defaults` has a row. Optional limits have a switch: off means no limit
//! (`None`). Turning on a limit core leaves unset starts the field at a `START_*` value below;
//! that is only where the field starts, the user picks the value. Each change is saved to
//! settings.json and pushed to the backend (`AppState::update_defaults`).

use std::time::Duration;

use colab_playground_core::{
    Defaults, provision::MAX_ATTEMPTS_CAP, provision::OnExhausted, sessions::Accelerator,
};
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    forms::{Choice, Input, NumberInput, Select, Switch},
    settings::{SettingsRow, SettingsSection},
    theme::{ActiveTheme, ControlSize},
    typography::Caption,
};
use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, SharedString, Styled, div};

use crate::{
    backend, format,
    i18n::{keys, tr, tr_args},
    platform,
    state::AppState,
    ui::{Inputs, tagged},
};

/// Where an idle limit starts when turned on.
pub const START_IDLE: Duration = Duration::from_secs(30 * 60);
/// Where a spending cap starts when turned on, in compute units.
pub const START_SPEND_CU: f64 = 10.0;
/// Where a balance floor starts when turned on, in compute units.
pub const START_MIN_BALANCE_CU: f64 = 5.0;
/// Where a minimum GPU memory starts when turned on, in GiB.
pub const START_MIN_VRAM_GIB: u32 = 40;
/// The most runtimes per account the field accepts (input sanity, not a policy).
pub const MAX_CONCURRENT_FIELD: f64 = 20.0;
/// The largest compute-unit amount the fields accept.
pub const MAX_CU_FIELD: f64 = 100_000.0;
/// The largest GPU memory the field accepts, in GiB.
pub const MAX_VRAM_FIELD_GIB: f64 = 1024.0;

/// The selector around a defaults row's control: `def <field>`.
pub fn field_selector(field: &str) -> String {
    format!("def {field}")
}

/// The selector around an optional limit's switch: `def-switch <field>`.
pub fn switch_selector(field: &str) -> String {
    format!("def-switch {field}")
}

type Edit<T> = fn(&mut Defaults, T);

fn edit<T: 'static>(
    handle: &Entity<AppState>,
    apply: Edit<T>,
) -> impl Fn(T, &mut gpui::Window, &mut App) + 'static {
    let handle = handle.clone();
    move |value, _, cx| {
        handle.update(cx, |state, cx| {
            state.update_defaults(|d| apply(d, value), cx)
        })
    }
}

fn row(
    field: &str,
    title: &str,
    hint: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> SettingsRow {
    let row = SettingsRow::new(tr(cx, title)).control(tagged(field_selector(field), control));
    match hint {
        Some(hint) => row.description(hint),
        None => row,
    }
}

/// An optional limit: a switch, and the value's field while it is on.
fn optional(
    field: &'static str,
    on: bool,
    toggle: impl Fn(bool, &mut gpui::Window, &mut App) + 'static,
    value: AnyElement,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(tagged(
            switch_selector(field),
            Switch::new(SharedString::from(format!("def-switch-{field}")), on)
                .name(tr(cx, keys::DEF_LIMIT))
                .on_change(toggle),
        ))
        .child(if on {
            value
        } else {
            Caption::new(tr(cx, keys::DEF_NO_LIMIT)).into_any_element()
        })
}

fn duration_field(
    id: &'static str,
    value: Duration,
    seconds: bool,
    on_change: impl Fn(Duration, &mut gpui::Window, &mut App) + 'static,
) -> AnyElement {
    let field = crate::ui::duration_field::DurationField::new(id, value, on_change);
    if seconds { field.seconds() } else { field }.into_any_element()
}

fn cu_field(
    id: &'static str,
    value: f64,
    on_change: impl Fn(f64, &mut gpui::Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    div()
        .w(cx.theme().menu_width())
        .child(
            NumberInput::new(id, value)
                .range(0.0, MAX_CU_FIELD)
                .precision(1)
                .step(1.0)
                .suffix(div().child(tr(cx, keys::UNIT_CU)))
                .on_change(on_change),
        )
        .into_any_element()
}

pub fn defaults_section(state: &AppState, handle: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let d = state.defaults();
    let core = Defaults::default();
    let theme = cx.theme();

    let max_runtime = optional(
        "max_runtime",
        d.max_runtime.is_some(),
        edit(handle, move |d, on: bool| {
            d.max_runtime = on.then(|| Defaults::default().max_runtime.unwrap_or(START_IDLE));
        }),
        duration_field(
            "def-max-runtime",
            d.max_runtime
                .unwrap_or(core.max_runtime.unwrap_or(START_IDLE)),
            false,
            edit(handle, |d, v: Duration| d.max_runtime = Some(v)),
        ),
        cx,
    );
    let max_spend = optional(
        "max_spend",
        d.max_spend_cu.is_some(),
        edit(handle, |d, on: bool| {
            d.max_spend_cu = on.then_some(START_SPEND_CU)
        }),
        cu_field(
            "def-max-spend",
            d.max_spend_cu.unwrap_or(START_SPEND_CU),
            edit(handle, |d, v: f64| d.max_spend_cu = Some(v)),
            cx,
        ),
        cx,
    );
    let min_balance = optional(
        "min_balance",
        d.min_balance_cu.is_some(),
        edit(handle, |d, on: bool| {
            d.min_balance_cu = on.then_some(START_MIN_BALANCE_CU)
        }),
        cu_field(
            "def-min-balance",
            d.min_balance_cu.unwrap_or(START_MIN_BALANCE_CU),
            edit(handle, |d, v: f64| d.min_balance_cu = Some(v)),
            cx,
        ),
        cx,
    );
    let idle = optional(
        "idle_timeout",
        d.idle_timeout.is_some(),
        edit(handle, |d, on: bool| {
            d.idle_timeout = on.then_some(START_IDLE)
        }),
        duration_field(
            "def-idle",
            d.idle_timeout.unwrap_or(START_IDLE),
            false,
            edit(handle, |d, v: Duration| d.idle_timeout = Some(v)),
        ),
        cx,
    );
    let concurrent = div().w(theme.menu_width()).child(
        NumberInput::new("def-concurrent", f64::from(d.max_concurrent_sessions))
            .range(1.0, MAX_CONCURRENT_FIELD)
            .precision(0)
            .step(1.0)
            .on_change(edit(handle, |d, v: f64| {
                d.max_concurrent_sessions = v.round() as u32
            })),
    );
    let attempts = div().w(theme.menu_width()).child(
        NumberInput::new("def-attempts", f64::from(d.max_attempts))
            .range(1.0, f64::from(MAX_ATTEMPTS_CAP))
            .precision(0)
            .step(1.0)
            .on_change(edit(handle, |d, v: f64| d.max_attempts = v.round() as u32)),
    );
    let exhausted = div().w(theme.menu_width()).child(
        Select::new(
            "def-on-exhausted",
            [
                Choice::new("stop", tr(cx, keys::DEF_EXHAUSTED_STOP)),
                Choice::new("keep", tr(cx, keys::DEF_EXHAUSTED_KEEP)),
            ],
        )
        .label(tr(cx, keys::DEF_ON_EXHAUSTED))
        .selected(match d.on_exhausted {
            OnExhausted::StopAndFail => "stop",
            OnExhausted::KeepLast => "keep",
        })
        .on_change({
            let handle = handle.clone();
            move |value, _, cx| {
                let choice = if value.as_ref() == "keep" {
                    OnExhausted::KeepLast
                } else {
                    OnExhausted::StopAndFail
                };
                handle.update(cx, |state, cx| {
                    state.update_defaults(|d| d.on_exhausted = choice, cx)
                })
            }
        }),
    );
    let accelerator = div().w(theme.menu_width()).child(
        Select::new(
            "def-accelerator",
            Accelerator::ALL.map(|a| Choice::new(a.as_str(), tr(cx, format::accelerator_key(a)))),
        )
        .label(tr(cx, keys::DEF_ACCELERATOR))
        .selected(d.accelerator.as_str())
        .on_change({
            let handle = handle.clone();
            move |value, _, cx| {
                if let Some(a) = Accelerator::ALL
                    .into_iter()
                    .find(|a| a.as_str() == value.as_ref())
                {
                    handle.update(cx, |state, cx| {
                        state.update_defaults(|d| d.accelerator = a, cx)
                    })
                }
            }
        }),
    );
    let high_mem = Switch::new("def-high-mem", d.high_mem)
        .name(tr(cx, keys::DEF_HIGH_MEM))
        .on_change(edit(handle, |d, on: bool| d.high_mem = on));
    let min_vram = optional(
        "min_vram",
        d.min_vram_mib.is_some(),
        edit(handle, |d, on: bool| {
            d.min_vram_mib = on.then_some(START_MIN_VRAM_GIB * 1024)
        }),
        div()
            .w(theme.menu_width())
            .child(
                NumberInput::new(
                    "def-min-vram",
                    f64::from(d.min_vram_mib.unwrap_or(START_MIN_VRAM_GIB * 1024) / 1024),
                )
                .range(1.0, MAX_VRAM_FIELD_GIB)
                .precision(0)
                .step(1.0)
                .suffix(div().child(tr(cx, keys::UNIT_GB)))
                .on_change(edit(handle, |d, v: f64| {
                    d.min_vram_mib = Some((v.round() as u32).max(1) * 1024)
                })),
            )
            .into_any_element(),
        cx,
    );
    let reset = {
        let handle = handle.clone();
        tagged(
            "def-reset",
            Button::new("def-reset", tr(cx, keys::DEF_RESET))
                .variant(ButtonVariant::Ghost)
                .size(ControlSize::Sm)
                .disabled(*d == core)
                .on_click(move |_, _, cx| handle.update(cx, |state, cx| state.reset_defaults(cx))),
        )
    };
    SettingsSection::new(tr(cx, keys::DEFAULTS_TITLE))
        .description(tr(cx, keys::DEFAULTS_DESCRIPTION))
        .row(row(
            "max_runtime",
            keys::DEF_MAX_RUNTIME,
            Some(tr(cx, keys::DEF_MAX_RUNTIME_HINT)),
            max_runtime,
            cx,
        ))
        .row(row(
            "max_spend",
            keys::DEF_MAX_SPEND,
            Some(tr(cx, keys::DEF_MAX_SPEND_HINT)),
            max_spend,
            cx,
        ))
        .row(row(
            "min_balance",
            keys::DEF_MIN_BALANCE,
            Some(tr(cx, keys::DEF_MIN_BALANCE_HINT)),
            min_balance,
            cx,
        ))
        .row(row(
            "idle_timeout",
            keys::DEF_IDLE,
            Some(tr(cx, keys::DEF_IDLE_HINT)),
            idle,
            cx,
        ))
        .row(row(
            "max_concurrent",
            keys::DEF_CONCURRENT,
            Some(tr(cx, keys::DEF_CONCURRENT_HINT)),
            concurrent,
            cx,
        ))
        .row(row(
            "max_attempts",
            keys::DEF_ATTEMPTS,
            Some(tr_args(
                cx,
                keys::DEF_ATTEMPTS_HINT,
                &[("cap", &MAX_ATTEMPTS_CAP.to_string())],
            )),
            attempts,
            cx,
        ))
        .row(row(
            "retry_delay",
            keys::DEF_RETRY_DELAY,
            None,
            duration_field(
                "def-retry-delay",
                d.retry_delay,
                true,
                edit(handle, |d, v: Duration| d.retry_delay = v),
            ),
            cx,
        ))
        .row(row(
            "on_exhausted",
            keys::DEF_ON_EXHAUSTED,
            None,
            exhausted,
            cx,
        ))
        .row(row(
            "accelerator",
            keys::DEF_ACCELERATOR,
            None,
            accelerator,
            cx,
        ))
        .row(row("high_mem", keys::DEF_HIGH_MEM, None, high_mem, cx))
        .row(row(
            "min_vram",
            keys::DEF_MIN_VRAM,
            Some(tr(cx, keys::DEF_MIN_VRAM_HINT)),
            min_vram,
            cx,
        ))
        .row(row(
            "winddown",
            keys::DEF_WINDDOWN,
            Some(tr(cx, keys::DEF_WINDDOWN_HINT)),
            duration_field(
                "def-winddown",
                d.winddown_grace,
                true,
                edit(handle, |d, v: Duration| d.winddown_grace = v),
            ),
            cx,
        ))
        .row(row(
            "poll_active",
            keys::DEF_POLL_ACTIVE,
            Some(tr(cx, keys::DEF_POLL_HINT)),
            duration_field(
                "def-poll-active",
                d.usage_poll_active,
                true,
                edit(handle, |d, v: Duration| d.usage_poll_active = v),
            ),
            cx,
        ))
        .row(row(
            "poll_idle",
            keys::DEF_POLL_IDLE,
            None,
            duration_field(
                "def-poll-idle",
                d.usage_poll_idle,
                true,
                edit(handle, |d, v: Duration| d.usage_poll_idle = v),
            ),
            cx,
        ))
        .row(SettingsRow::new(tr(cx, keys::DEF_RESET)).control(reset))
}

/// Where the CLI lives: the WSL distribution (Windows) and the `colab` paths.
pub fn host_section(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let simulated = backend::shows_demo_badge(state.flavor());
    let busy = !state.can_change_host();
    let mut section =
        SettingsSection::new(tr(cx, keys::HOST_TITLE)).description(tr(cx, keys::HOST_DESCRIPTION));
    if platform::uses_wsl() {
        let distros: Vec<String> = state
            .probe()
            .ready()
            .map(|report| report.wsl_distros.clone())
            .unwrap_or_default();
        let current = state.settings().wsl_distro.clone();
        let mut choices = vec![Choice::new("", tr(cx, keys::HOST_WSL_DEFAULT))];
        choices.extend(distros.iter().map(|d| Choice::new(d.clone(), d.clone())));
        if let Some(current) = current.as_ref().filter(|c| !distros.contains(c)) {
            choices.push(Choice::new(current.clone(), current.clone()));
        }
        let pick = handle.clone();
        section = section.row(row(
            "wsl_distro",
            keys::HOST_WSL,
            None,
            div().w(theme.menu_width()).child(
                Select::new("host-wsl", choices)
                    .label(tr(cx, keys::HOST_WSL))
                    .selected(current.unwrap_or_default())
                    .disabled(busy)
                    .on_change(move |value, _, cx| {
                        let distro = (!value.is_empty()).then(|| value.to_string());
                        pick.update(cx, |state, cx| state.set_wsl_distro(distro, cx));
                    }),
            ),
            cx,
        ));
    }
    let path = |input: &Entity<ely_gpui_component::forms::TextInput>| {
        div()
            .w(theme.container_width(ely_gpui_component::theme::ContainerSize::Sm) * 0.5)
            .child(Input::new(input))
    };
    let (apply, colab, python) = (
        handle.clone(),
        inputs.colab_path.clone(),
        inputs.colab_python.clone(),
    );
    section = section
        .row(row(
            "colab_path",
            keys::HOST_COLAB_PATH,
            None,
            path(&inputs.colab_path),
            cx,
        ))
        .row(row(
            "colab_python",
            keys::HOST_PYTHON_PATH,
            Some(tr(cx, keys::HOST_PYTHON_HINT)),
            path(&inputs.colab_python),
            cx,
        ))
        .row(
            SettingsRow::new(tr(cx, keys::HOST_APPLY))
                .description(if simulated {
                    tr(cx, keys::HOST_MOCK_NOTE)
                } else if busy {
                    tr(cx, keys::HOST_BUSY)
                } else {
                    tr(cx, keys::HOST_DESCRIPTION)
                })
                .control(tagged(
                    "host-apply",
                    Button::new("host-apply", tr(cx, keys::HOST_APPLY))
                        .size(ControlSize::Sm)
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            let read = |input: &Entity<ely_gpui_component::forms::TextInput>,
                                        cx: &App| {
                                let text = input.read(cx).text().trim().to_owned();
                                (!text.is_empty()).then_some(text)
                            };
                            let (colab, python) = (read(&colab, cx), read(&python, cx));
                            if colab.is_none() && python.is_some() {
                                apply.update(cx, |state, cx| state.warn_host_paths(cx));
                                return;
                            }
                            apply.update(cx, |state, cx| state.set_toolchain(colab, python, cx));
                        }),
                )),
        );
    section
}

/// Selector of the session-limit card shown at the top of Settings from the New session
/// dialog.
pub const LIMIT_FOCUS_SELECTOR: &str = "settings-limit-focus";

/// The session limit, called out at the top of Settings when the New session dialog sends
/// the user here.
pub fn limit_focus(state: &AppState, handle: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let d = state.defaults();
    tagged(
        LIMIT_FOCUS_SELECTOR,
        ely_gpui_component::layout::Card::new()
            .header(
                ely_gpui_component::layout::CardHeader::new(tr(cx, keys::DEF_CONCURRENT))
                    .description(tr(cx, keys::LIMIT_FOCUS_HINT)),
            )
            .child(
                div().w(theme.menu_width()).child(
                    NumberInput::new("def-concurrent-focus", f64::from(d.max_concurrent_sessions))
                        .range(1.0, MAX_CONCURRENT_FIELD)
                        .precision(0)
                        .step(1.0)
                        .on_change(edit(handle, |d, v: f64| {
                            d.max_concurrent_sessions = v.round() as u32
                        })),
                ),
            ),
    )
    .w_full()
}
