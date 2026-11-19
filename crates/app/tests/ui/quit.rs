//! Closing before the app knows what runs: the quit prompt waits for the check, then quits if
//! nothing bills, or stays and shows what bills or that the app could not check.
//!
//! Start-up runs without a window ([`Launch`]), since opening one runs the executor and the
//! start-up loads would arrive first. The prompt's wording is checked in a window, closing
//! after a failed listing (the close lists again and waits for that).

use colab_playground::{
    state::{AppState, Overlay, QuitPrompt},
    ui::overlays::{QUIT_CHECKING, QUIT_LEAVE, QUIT_STOP_ALL, QUIT_UNCHECKED},
};
use colab_playground_core::mock::{MockAccount, MockFailures, MockScenario};
use gpui::{Entity, TestAppContext};

use crate::support::{Launch, Options, TestApp};

fn quit_prompt(state: &AppState) -> Option<QuitPrompt> {
    match state.overlay() {
        Some(Overlay::Quit(prompt)) => Some(prompt.clone()),
        _ => None,
    }
}

fn listing_fails() -> MockFailures {
    MockFailures {
        runtimes_listing_fails: true,
        ..MockFailures::default()
    }
}

/// Closes the app right after launch, before anything it checks has arrived.
fn close_at_launch(cx: &mut TestAppContext, state: &Entity<AppState>) {
    state.update(cx, |state, cx| {
        assert!(state.billing().checking, "nothing has loaded yet");
        state.request_quit(cx);
        let prompt = quit_prompt(state).expect("the quit prompt opens while the app checks");
        assert!(prompt.waiting);
        assert!(!state.quit_approved());
    });
}

#[gpui::test]
fn closing_at_launch_quits_once_the_check_finds_nothing(cx: &mut TestAppContext) {
    let launch = Launch::new(cx, Options::scenario(MockScenario::single()));
    close_at_launch(cx, &launch.state);
    cx.run_until_parked();
    launch.state.read_with(cx, |state, _| {
        assert!(state.quit_approved(), "no second click needed");
        assert!(quit_prompt(state).is_none());
    });
}

#[gpui::test]
fn closing_at_launch_with_runtimes_billing_keeps_the_prompt(cx: &mut TestAppContext) {
    let launch = Launch::new(cx, Options::default());
    close_at_launch(cx, &launch.state);
    cx.run_until_parked();
    launch.state.read_with(cx, |state, _| {
        assert!(!state.quit_approved());
        let prompt = quit_prompt(state).expect("the prompt stays");
        assert!(!prompt.waiting, "the check is over");
        let billing = state.billing();
        assert!(!billing.checking);
        assert_eq!(billing.watched.len() + billing.unwatched.len(), 4);
    });
}

#[gpui::test]
fn closing_at_launch_when_the_listing_fails_keeps_the_prompt(cx: &mut TestAppContext) {
    let options = Options {
        failures: listing_fails(),
        ..Options::scenario(MockScenario::single())
    };
    let launch = Launch::new(cx, options);
    close_at_launch(cx, &launch.state);
    cx.run_until_parked();
    launch.state.read_with(cx, |state, _| {
        assert!(!state.quit_approved(), "it could not tell, so it asks");
        assert!(!quit_prompt(state).expect("the prompt stays").waiting);
        let billing = state.billing();
        assert!(billing.unknown && !billing.checking && !billing.known());
    });
}

#[gpui::test]
fn cancelling_at_launch_while_checking_keeps_the_app_open(cx: &mut TestAppContext) {
    let launch = Launch::new(cx, Options::scenario(MockScenario::single()));
    close_at_launch(cx, &launch.state);
    // Cancel before the check is done.
    launch.state.update(cx, |state, cx| state.close_overlay(cx));
    cx.run_until_parked();
    launch.state.read_with(cx, |state, _| {
        assert!(!state.quit_approved(), "the user changed their mind");
        assert!(quit_prompt(state).is_none());
    });
}

#[gpui::test]
fn closing_while_the_first_listing_runs_waits_for_that_one_only(cx: &mut TestAppContext) {
    let options = Options {
        failures: listing_fails(),
        ..Options::scenario(MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    app.set_failures(MockFailures::default());
    let before = app.mock.runtime_listings();
    app.update(|state, cx| {
        // No list yet and a listing on its way, as at launch.
        state.load_runtimes(cx);
        state.request_quit(cx);
        assert!(quit_prompt(state).expect("the prompt waits").waiting);
    });
    assert!(app.read(|state, _| state.quit_approved()));
    assert_eq!(
        app.mock.runtime_listings(),
        before + 1,
        "the close changed nothing, so it asks for no other listing"
    );
}

/// The app with a failed listing, then `failures` for the listing the close starts.
fn close_after_a_failed_listing(
    cx: &mut TestAppContext,
    scenario: MockScenario,
    failures: MockFailures,
) -> TestApp<'_> {
    let options = Options {
        failures: listing_fails(),
        ..Options::scenario(scenario)
    };
    let mut app = TestApp::with(cx, options);
    assert!(app.read(|state, _| state.runtimes().error().is_some()));
    app.set_failures(failures);
    assert!(
        !app.cx.simulate_close(),
        "the close waits for a fresh listing"
    );
    assert!(app.read(|state, _| quit_prompt(state).expect("the prompt waits").waiting));
    app.settle();
    app
}

#[gpui::test]
fn closing_after_a_failed_listing_lists_again_and_quits(cx: &mut TestAppContext) {
    let mut app = close_after_a_failed_listing(cx, MockScenario::single(), MockFailures::default());
    assert!(
        app.read(|state, _| state.quit_approved()),
        "the fresh listing found nothing"
    );
}

#[gpui::test]
fn the_prompt_lists_what_the_check_found(cx: &mut TestAppContext) {
    let mut app = close_after_a_failed_listing(cx, MockScenario::demo(), MockFailures::default());
    assert!(!app.read(|state, _| state.quit_approved()));
    assert!(app.shown("quit-runtime a100-7f3a"));
    assert!(app.shown("quit-unmanaged L4"));
    assert!(!app.shown(QUIT_CHECKING), "no longer checking");
    assert!(!app.shown(QUIT_UNCHECKED), "every account was listed");
}

#[gpui::test]
fn the_prompt_says_when_it_could_not_check(cx: &mut TestAppContext) {
    let mut app = close_after_a_failed_listing(cx, MockScenario::single(), listing_fails());
    assert!(!app.read(|state, _| state.quit_approved()));
    assert!(app.shown(QUIT_UNCHECKED), "it says it could not check");
    assert!(!app.shown(QUIT_CHECKING));
    // The same choices: "Stop all" lists again first, and says so when that fails too.
    app.click(QUIT_STOP_ALL);
    assert!(!app.read(|state, _| state.quit_approved()));
    assert!(app.shown("quit-failure"));
    app.click(QUIT_LEAVE);
    assert!(app.read(|state, _| state.quit_approved()));
}

#[gpui::test]
fn the_prompt_says_when_an_account_could_not_be_listed(cx: &mut TestAppContext) {
    let scenario = MockScenario {
        accounts: vec![
            MockAccount {
                email: "ada@example.com".into(),
                balance_cu: 100.0,
            },
            MockAccount {
                email: "lin@example.com".into(),
                balance_cu: 50.0,
            },
        ],
        runtimes: vec![],
    };
    let failures = MockFailures {
        unlisted_accounts: vec!["lin@example.com".into()],
        ..MockFailures::default()
    };
    let mut app = close_after_a_failed_listing(cx, scenario, failures);
    assert!(
        !app.read(|state, _| state.quit_approved()),
        "lin@example.com may still have runtimes"
    );
    assert!(app.shown(QUIT_UNCHECKED));
}

#[gpui::test]
fn a_later_listing_that_finds_nothing_still_quits(cx: &mut TestAppContext) {
    let mut app = close_after_a_failed_listing(cx, MockScenario::single(), listing_fails());
    assert!(
        app.shown(QUIT_UNCHECKED),
        "the prompt says it could not check"
    );
    // Any later listing (a refresh, or the one after a failed "Stop all") settles it too.
    app.set_failures(MockFailures::default());
    app.update(|state, cx| state.load_runtimes(cx));
    app.settle();
    assert!(
        app.read(|state, _| state.quit_approved()),
        "the user asked to close and nothing bills"
    );
}

#[gpui::test]
fn after_a_failed_stop_all_a_listing_that_finds_nothing_quits(cx: &mut TestAppContext) {
    let mut app = close_after_a_failed_listing(cx, MockScenario::single(), listing_fails());
    app.click(QUIT_STOP_ALL);
    assert!(app.shown("quit-failure"), "Stop all could not list either");
    assert!(!app.read(|state, _| state.quit_approved()));
    app.set_failures(MockFailures::default());
    app.update(|state, cx| state.load_runtimes(cx));
    app.settle();
    assert!(app.read(|state, _| state.quit_approved()));
}

#[gpui::test]
fn while_checking_the_prompt_says_it_closes_by_itself(cx: &mut TestAppContext) {
    let options = Options {
        failures: listing_fails(),
        ..Options::scenario(MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    app.set_failures(MockFailures::default());
    assert!(!app.cx.simulate_close());
    // Drawn before the fresh listing arrives.
    app.draw_now();
    assert!(app.shown(QUIT_CHECKING));
    assert!(!app.shown(QUIT_UNCHECKED));
    app.settle();
    assert!(app.read(|state, _| state.quit_approved()));
}
