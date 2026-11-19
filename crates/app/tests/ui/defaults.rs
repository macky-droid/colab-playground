//! Defaults: edits reach settings.json and the backend; saved values are pushed at startup.

use std::time::Duration;

use colab_playground::{
    i18n::keys,
    route::Route,
    ui::pages::defaults::{START_SPEND_CU, switch_selector},
};
use colab_playground_core::{Defaults, backend::Backend, sessions::Accelerator};
use gpui::TestAppContext;

use crate::support::{ConfigDir, Options, TestApp};

#[gpui::test]
fn edited_defaults_are_saved_and_reach_the_backend(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.update(|state, cx| {
        state.update_defaults(
            |defaults| {
                defaults.max_attempts = 5;
                defaults.idle_timeout = Some(Duration::from_secs(15 * 60));
                defaults.accelerator = Accelerator::L4;
            },
            cx,
        )
    });
    let expected = Defaults {
        max_attempts: 5,
        idle_timeout: Some(Duration::from_secs(15 * 60)),
        accelerator: Accelerator::L4,
        ..Defaults::default()
    };
    assert_eq!(
        app.saved_settings().map(|s| s.defaults),
        Some(expected.clone())
    );
    assert_eq!(app.mock.defaults(), expected, "pushed to the backend");
}

#[gpui::test]
fn turning_on_a_limit_in_settings_saves_it(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.click(Route::Settings.nav_selector());
    app.click(&switch_selector("max_spend"));
    assert_eq!(
        app.read(|state, _| state.defaults().max_spend_cu),
        Some(START_SPEND_CU)
    );
    assert_eq!(
        app.saved_settings().and_then(|s| s.defaults.max_spend_cu),
        Some(START_SPEND_CU)
    );
    assert_eq!(app.mock.defaults().max_spend_cu, Some(START_SPEND_CU));

    app.click("def-reset");
    assert_eq!(
        app.read(|state, _| state.defaults().clone()),
        Defaults::default()
    );
    assert_eq!(app.mock.defaults(), Defaults::default());
}

#[gpui::test]
fn a_colab_path_alone_is_enough_and_an_interpreter_alone_is_refused(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.click(Route::Settings.nav_selector());
    app.fill(
        |inputs| inputs.colab_python.clone(),
        "/opt/colab/bin/python",
    );
    app.click("host-apply");
    let warning = app.text(keys::HOST_INTERPRETER_ALONE);
    assert!(app.toasts().contains(&warning), "{:?}", app.toasts());
    assert_eq!(app.saved_settings().and_then(|s| s.colab_python), None);

    // The interpreter is derived from the colab file when left empty (core's
    // `Toolchain::from_colab_path`).
    app.fill(|inputs| inputs.colab_python.clone(), "");
    app.fill(|inputs| inputs.colab_path.clone(), "/opt/colab/bin/colab");
    app.click("host-apply");
    let saved = app.saved_settings().expect("saved");
    assert_eq!(saved.colab_path.as_deref(), Some("/opt/colab/bin/colab"));
    assert_eq!(saved.colab_python, None);
    let toolchain = saved.host_setup().toolchain.expect("a toolchain");
    assert!(
        toolchain.needs_resolution(),
        "the interpreter is found later"
    );
    assert!(
        app.read(|state, _| state.backend_error().is_none()),
        "the simulation ignores the paths and keeps running"
    );
}

#[gpui::test]
fn saved_defaults_are_pushed_to_the_backend_at_startup(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith(
            r#"{ "defaults": { "max_attempts": 6, "retry_delay": 45, "high_mem": true } }"#.into(),
        ),
        ..Options::default()
    };
    let app = TestApp::with(cx, options);
    let pushed = app.mock.defaults();
    assert_eq!(pushed.max_attempts, 6);
    assert_eq!(pushed.retry_delay, Duration::from_secs(45));
    assert!(pushed.high_mem);
}

#[gpui::test]
fn a_bad_saved_default_is_reset_and_reported(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith(r#"{ "defaults": { "max_attempts": "many" } }"#.into()),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    let title = app.text(colab_playground::i18n::keys::TOAST_DEFAULTS_RESET_TITLE);
    assert!(app.toasts().contains(&title), "{:?}", app.toasts());
    assert_eq!(
        app.mock.defaults().max_attempts,
        Defaults::default().max_attempts
    );
}
