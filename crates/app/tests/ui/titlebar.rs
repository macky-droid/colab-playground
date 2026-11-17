//! The app's own title strip: present, lined up with the sidebar, and with the right window
//! controls for each OS. The per-OS layout is a pure function of `platform::Chrome`, so every
//! variant is drawn and checked here on any host.

use colab_playground::{
    i18n::{self, Language},
    platform::{self, Chrome},
    route::Route,
    ui::{
        sidebar::ACCOUNT_SLOT_SELECTOR,
        titlebar::{self, BRAND_SELECTOR, CONTROLS_SELECTOR, MARK_SELECTOR, TITLE_BAR_SELECTOR},
    },
};
use ely_gpui_component::{
    primitives::FocusScope,
    theme::{ActiveTheme, Platform},
};
use gpui::{
    Context, Decorations, FocusHandle, IntoElement, ParentElement, Render, Styled, TestAppContext,
    Window,
};

use colab_playground_core::mock::{MockAccount, MockScenario};

use crate::support::{Options, TestApp, mount};

const CAPTION_BUTTONS: [&str; 3] = ["caption-Minimize", "caption-Maximize", "caption-Close"];

struct Strip {
    chrome: Chrome,
    focus: FocusHandle,
}

impl Render for Strip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        FocusScope::new(&self.focus)
            .root()
            .size_full()
            .child(titlebar::render(self.chrome, window, cx))
    }
}

#[gpui::test]
fn every_chrome_draws_its_strip_with_only_its_own_controls(cx: &mut TestAppContext) {
    for chrome in Chrome::ALL {
        let layout = chrome.layout();
        let (_, cx) = mount(cx, move |_, cx| {
            i18n::install(Language::EnUs, cx);
            Strip {
                chrome,
                focus: cx.focus_handle(),
            }
        });
        let strip = cx
            .debug_bounds(TITLE_BAR_SELECTOR)
            .unwrap_or_else(|| panic!("{chrome:?} draws the title strip"));
        let height =
            cx.update(|window, cx| cx.theme().titlebar_height().to_pixels(window.rem_size()));
        assert_eq!(
            strip.size.height, height,
            "{chrome:?}: the theme's title-bar height"
        );
        assert!(cx.debug_bounds(BRAND_SELECTOR).is_some());
        assert_eq!(
            cx.debug_bounds(CONTROLS_SELECTOR).is_some(),
            layout.controls.is_some(),
            "{chrome:?}: app-drawn window controls"
        );
        for button in CAPTION_BUTTONS {
            assert_eq!(
                cx.debug_bounds(button).is_some(),
                layout.controls == Some(Platform::Windows),
                "{chrome:?}: {button}"
            );
        }
        if let Some(controls) = cx.debug_bounds(CONTROLS_SELECTOR) {
            assert_eq!(
                controls.top(),
                strip.top(),
                "{chrome:?}: controls hug the top"
            );
            assert_eq!(
                controls.right(),
                strip.right(),
                "{chrome:?}: and the right edge"
            );
        }
        assert_eq!(
            cx.debug_bounds(MARK_SELECTOR).is_some(),
            layout.mark,
            "{chrome:?}: the mark and name, except beside macOS's traffic lights"
        );
    }
}

#[gpui::test]
fn the_app_window_carries_the_strip_for_this_os(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    assert!(app.shown(TITLE_BAR_SELECTOR));
    // gpui's test window reports server decorations, as Windows and macOS windows do.
    let layout = platform::chrome(Decorations::Server).layout();
    assert_eq!(app.shown(CONTROLS_SELECTOR), layout.controls.is_some());
    assert_eq!(app.shown(MARK_SELECTOR), layout.mark);
    for button in CAPTION_BUTTONS {
        assert_eq!(
            app.shown(button),
            layout.controls == Some(Platform::Windows)
        );
    }
}

#[gpui::test]
fn the_mark_and_the_account_slot_line_up_with_the_nav_items(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    let nav = app
        .bounds(Route::Overview.nav_selector())
        .expect("a nav item");
    let account = app.bounds(ACCOUNT_SLOT_SELECTOR).expect("the account slot");
    let strip = app.bounds(TITLE_BAR_SELECTOR).expect("the strip");
    let brand = app.bounds(BRAND_SELECTOR).expect("the brand column");
    assert_eq!(
        account.left(),
        nav.left(),
        "the account slot starts where nav items do"
    );
    assert_eq!(
        account.size.width, nav.size.width,
        "and spans the same width"
    );
    // Drawn except on macOS (`the_app_window_carries_the_strip_for_this_os`).
    if let Some(mark) = app.bounds(MARK_SELECTOR) {
        assert_eq!(mark.left(), nav.left(), "the mark sits over the nav icons");
    }
    assert!(
        nav.top() >= strip.bottom(),
        "the sidebar starts under the strip"
    );
    assert!(
        brand.right() > nav.right(),
        "the brand column is the sidebar's width"
    );
}

#[gpui::test]
fn a_long_email_stays_inside_the_account_slot(cx: &mut TestAppContext) {
    let email = "a.very.long.name.for.an.account@an-extremely-long-domain.example.com";
    let scenario = MockScenario {
        accounts: vec![MockAccount {
            email: email.into(),
            balance_cu: 10.0,
        }],
        runtimes: vec![],
    };
    let mut app = TestApp::with(cx, Options::scenario(scenario));
    assert_eq!(
        app.read(|state, _| state.active_email()).as_deref(),
        Some(email)
    );
    let nav = app
        .bounds(Route::Overview.nav_selector())
        .expect("a nav item");
    let account = app.bounds(ACCOUNT_SLOT_SELECTOR).expect("the account slot");
    assert_eq!(account.left(), nav.left());
    assert_eq!(
        account.size.width, nav.size.width,
        "the email is cut short, not widening the slot"
    );
}

/// Opens the "Cancel auto-stop?" confirmation on the demo's A100 session.
fn open_a_dialog(app: &mut TestApp<'_>) {
    app.click(Route::Sessions.nav_selector());
    app.click(&colab_playground::ui::pages::sessions::cancel_auto_stop_selector("a100-7f3a"));
    assert!(matches!(
        app.read(|state, _| state.overlay().cloned()),
        Some(colab_playground::state::Overlay::CancelAutoStop(_))
    ));
}

fn quit_prompt_open(app: &mut TestApp<'_>) -> bool {
    matches!(
        app.read(|state, _| state.overlay().cloned()),
        Some(colab_playground::state::Overlay::Quit(_))
    )
}

// The drawn caption buttons are the Windows chrome; macOS keeps native traffic lights.
#[cfg_attr(
    not(target_os = "windows"),
    ignore = "drawn caption buttons are Windows chrome"
)]
#[gpui::test]
fn with_a_dialog_open_one_click_on_close_asks_before_quitting(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    open_a_dialog(&mut app);
    app.click("caption-Close");
    assert!(
        quit_prompt_open(&mut app),
        "the first click runs the close check, not just dismissing the dialog"
    );
    app.click("caption-Close");
    assert!(
        quit_prompt_open(&mut app),
        "a second click keeps the same prompt"
    );
    assert!(!app.read(|state, _| state.quit_approved()));
}

#[cfg_attr(
    not(target_os = "windows"),
    ignore = "drawn caption buttons are Windows chrome"
)]
#[gpui::test]
fn with_a_dialog_open_and_nothing_billing_one_click_quits(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    app.click(Route::Sessions.nav_selector());
    app.click(colab_playground::ui::pages::sessions::NEW_SESSION_SELECTOR);
    assert!(
        app.read(|state, _| state.overlay().is_some()),
        "a dialog is open"
    );
    let close = app.bounds("caption-Close").expect("the close button");
    // The app quits, so click without the harness's settle.
    app.cx
        .simulate_click(close.center(), gpui::Modifiers::none());
    app.cx.run_until_parked();
    assert!(
        app.state
            .read_with(&*app.cx, |state, _| state.quit_approved()),
        "one click quits"
    );
    assert!(
        app.state
            .read_with(&*app.cx, |state, _| state.overlay().is_none()),
        "no quit prompt"
    );
}

/// Clicks `control` with a dialog open. The test platform does not implement minimising or
/// maximising and panics when asked to, so that panic proves the click reached the control
/// rather than the dialog's backdrop.
fn reaches_the_control_over_a_dialog(cx: &mut TestAppContext, control: &str) {
    let mut app = TestApp::new(cx);
    open_a_dialog(&mut app);
    let bounds = app.bounds(control).expect("the control");
    let reached = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.cx
            .simulate_click(bounds.center(), gpui::Modifiers::none());
    }));
    assert!(
        reached.is_err(),
        "{control} was swallowed by the dialog's backdrop"
    );
}

#[cfg_attr(
    not(target_os = "windows"),
    ignore = "drawn caption buttons are Windows chrome"
)]
#[gpui::test]
fn minimise_works_with_a_dialog_open(cx: &mut TestAppContext) {
    reaches_the_control_over_a_dialog(cx, "caption-Minimize");
}

#[cfg_attr(
    not(target_os = "windows"),
    ignore = "drawn caption buttons are Windows chrome"
)]
#[gpui::test]
fn maximise_works_with_a_dialog_open(cx: &mut TestAppContext) {
    reaches_the_control_over_a_dialog(cx, "caption-Maximize");
}
