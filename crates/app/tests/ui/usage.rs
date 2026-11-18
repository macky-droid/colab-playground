//! Usage: the sidebar badge follows virtual time, goes stale when fetching fails, is never
//! shown as zero when unknown, and refreshes on request.

use std::time::{Duration, SystemTime};

use colab_playground::{
    format,
    i18n::{keys, tr, tr_args},
    route::Route,
    state::{POLL_CHECK, usage_is_stale, usage_stale_note},
    ui::{
        pages::accounts::{REFRESH_SELECTOR, TOTALS_SELECTOR, USAGE_ERROR_SELECTOR},
        sidebar::USAGE_SLOT_SELECTOR,
    },
};
use colab_playground_core::{
    mock::{MockFailures, MockScenario},
    usage::UsageSnapshot,
};
use gpui::{SharedString, TestAppContext};

use crate::support::{Options, TestApp};

fn active_snapshot(app: &mut TestApp<'_>) -> Option<UsageSnapshot> {
    app.read(|state, _| state.active_usage().and_then(|usage| usage.snapshot))
}

fn badge_text(app: &mut TestApp<'_>, snapshot: &UsageSnapshot) -> SharedString {
    app.cx.update(|_, cx| {
        tr_args(
            cx,
            keys::USAGE_BADGE,
            &[
                ("balance", &format::cu(cx, snapshot.balance_cu)),
                ("rate", &format::cu_rate(cx, snapshot.rate_cu_per_hour)),
            ],
        )
    })
}

#[gpui::test]
fn the_badge_burns_down_with_the_clock_and_turns_stale_when_fetching_fails(
    cx: &mut TestAppContext,
) {
    let mut app = TestApp::new(cx);
    let first = active_snapshot(&mut app).expect("usage is fetched at start");
    assert!(
        first.rate_cu_per_hour > 0.0,
        "the demo's runtimes are burning"
    );
    let text = badge_text(&mut app, &first);
    assert!(
        app.shown(&format!("{USAGE_SLOT_SELECTOR} {text}")),
        "{text}"
    );

    app.advance(Duration::from_secs(30 * 60));
    let later = active_snapshot(&mut app).expect("still known");
    assert!(later.fetched_at > first.fetched_at, "the poller refreshed");
    assert!(
        later.balance_cu < first.balance_cu,
        "half an hour of burning"
    );
    let text = badge_text(&mut app, &later);
    assert!(
        app.shown(&format!("{USAGE_SLOT_SELECTOR} {text}")),
        "{text}"
    );

    app.set_failures(MockFailures {
        usage_fetch_fails: true,
        ..MockFailures::default()
    });
    app.advance(Duration::from_secs(60));
    let stale = active_snapshot(&mut app).expect("the last values are kept");
    assert!(stale.stale, "a failed fetch marks the snapshot stale");
    assert_eq!(
        stale.balance_cu, later.balance_cu,
        "and keeps the last balance"
    );
    let outdated = app.text(keys::USAGE_OUTDATED);
    let text = format!("{} · {outdated}", badge_text(&mut app, &stale));
    assert!(
        app.shown(&format!("{USAGE_SLOT_SELECTOR} {text}")),
        "{text}"
    );
}

#[gpui::test]
fn without_a_reading_usage_is_unknown_not_zero(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            usage_fetch_fails: true,
            ..MockFailures::default()
        },
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert!(active_snapshot(&mut app).is_none());
    let unknown = app.text(keys::SHELL_USAGE);
    assert!(app.shown(&format!("{USAGE_SLOT_SELECTOR} {unknown}")));
    let summary = app.read(|state, _| state.usage_summary());
    assert_eq!((summary.known, summary.unknown), (0, 2));
    assert!(!app.shown("unmanaged-warning"));

    app.click(Route::Accounts.nav_selector());
    let unknown_total = app.cx.update(|_, cx| tr(cx, keys::SUMMARY_UNKNOWN));
    assert!(app.shown(&format!("{TOTALS_SELECTOR} {unknown_total}")));
}

#[gpui::test]
fn stale_unmanaged_warning_describes_the_last_reading(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    let email = app.read(|state, _| state.unmanaged()[0].0.clone().unwrap());
    let current = app
        .cx
        .update(|_, cx| tr_args(cx, keys::UNMANAGED_ONE, &[("email", &email)]));
    let current_title = app.text(keys::UNMANAGED_TITLE);
    assert!(app.shown(&format!("unmanaged-warning {current_title} {current}")));
    app.set_failures(MockFailures {
        usage_fetch_fails: true,
        ..MockFailures::default()
    });
    app.advance(Duration::from_secs(60));
    assert!(active_snapshot(&mut app).unwrap().stale);
    let historical = app
        .cx
        .update(|_, cx| tr_args(cx, keys::UNMANAGED_LAST_ONE, &[("email", &email)]));
    let title = app.text(keys::UNMANAGED_LAST_TITLE);
    assert!(app.shown(&format!("unmanaged-warning {title} {historical}")));
    assert!(!app.shown(&format!("unmanaged-warning {current_title} {current}")));
}

#[gpui::test]
fn a_refresh_that_fails_outright_is_shown_on_accounts(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.click(Route::Accounts.nav_selector());
    assert!(!app.shown(USAGE_ERROR_SELECTOR));
    app.set_failures(MockFailures {
        keyring_unavailable: true,
        ..MockFailures::default()
    });
    app.click(REFRESH_SELECTOR);
    assert!(app.read(|state, _| state.usage_error().is_some()));
    assert!(app.shown(USAGE_ERROR_SELECTOR), "not a silent failure");
}

#[test]
fn old_values_are_not_reported_as_a_failed_refresh() {
    let snapshot = UsageSnapshot {
        balance_cu: 10.0,
        rate_cu_per_hour: 0.0,
        assignments: 0,
        fetched_at: SystemTime::UNIX_EPOCH,
        stale: false,
    };
    let later = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(3600));
    assert!(usage_is_stale(&snapshot, later), "an hour old");
    assert_eq!(usage_stale_note(&snapshot), keys::USAGE_TIP_OLD);
    let failed = UsageSnapshot {
        stale: true,
        ..snapshot
    };
    assert_eq!(usage_stale_note(&failed), keys::USAGE_TIP_STALE);
}

#[gpui::test]
fn unverified_ownership_is_unknown_even_when_usage_is_fresh_or_stale(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    let email = app.read(|state, _| state.active_email().unwrap().to_string());
    let title = app.text(keys::UNMANAGED_UNKNOWN_TITLE);
    let body = app
        .cx
        .update(|_, cx| tr_args(cx, keys::UNMANAGED_UNKNOWN, &[("email", &email)]));
    app.set_failures(MockFailures {
        unlisted_accounts: vec![email.clone()],
        ..MockFailures::default()
    });
    app.advance(Duration::from_secs(60));
    assert!(!active_snapshot(&mut app).unwrap().stale);
    assert!(app.read(|state, _| {
        state
            .unmanaged()
            .iter()
            .any(|(e, count, _)| e.as_ref() == Some(&email) && count.is_none())
    }));
    assert!(app.shown(&format!("unmanaged-warning {title} {body}")));
    app.set_failures(MockFailures {
        unlisted_accounts: vec![email.clone()],
        usage_fetch_fails: true,
        ..MockFailures::default()
    });
    app.advance(Duration::from_secs(60));
    assert!(active_snapshot(&mut app).unwrap().stale);
    assert!(app.shown(&format!("unmanaged-warning {title} {body}")));
    app.set_failures(MockFailures::default());
    app.advance(Duration::from_secs(60));
    assert!(!app.shown(&format!("unmanaged-warning {title} {body}")));
}

#[gpui::test]
fn refresh_fetches_now_and_the_poller_waits_for_its_cadence(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    let first = active_snapshot(&mut app).expect("fetched at start");

    // Nothing runs on `single`, so the idle cadence (5 min) applies: one check is not enough.
    app.advance(POLL_CHECK);
    assert_eq!(
        active_snapshot(&mut app).map(|s| s.fetched_at),
        Some(first.fetched_at)
    );

    app.click(Route::Accounts.nav_selector());
    app.clock.advance(Duration::from_secs(10));
    app.click(REFRESH_SELECTOR);
    let refreshed = active_snapshot(&mut app).expect("still known");
    assert!(
        refreshed.fetched_at > first.fetched_at,
        "a manual refresh fetches at once"
    );
}
