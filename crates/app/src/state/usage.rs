//! Compute-unit usage: polling at core's cadence, the cache and what the UI shows.
//!
//! A foreground loop wakes every [`POLL_CHECK`] and, when idle, asks the backend on the
//! background executor whether a refresh is due (`usage_poll_due`, whose cadence comes from
//! `Defaults`) and refreshes if so. The backend's own clock (`now()`) comes back with every
//! result, so ages are measured on the same clock as `fetched_at`.

use std::time::{Duration, SystemTime};

use colab_playground_core::{
    Error, ErrorKind, accounts::AccountId, backend::AccountUsageView, guard::USAGE_STALE_AFTER,
    usage::UsageSnapshot,
};
use gpui::{Context, Task};

use super::{AccountDialog, AppState, Op};
use crate::i18n::keys;

/// How often the poller asks whether a usage refresh is due (the cadence itself is a policy
/// value in `Defaults`).
pub const POLL_CHECK: Duration = Duration::from_secs(5);

/// The last usage fetched and the backend's time then.
#[derive(Clone, Debug, Default)]
pub(crate) struct UsageCache {
    pub(crate) views: Vec<AccountUsageView>,
    pub(crate) now: Option<SystemTime>,
    pub(crate) error: Option<Error>,
}

/// Usage totals across accounts; unknown accounts are counted, never added as zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageSummary {
    /// Sum of known balances.
    pub balance_cu: f64,
    /// Sum of known burn rates.
    pub rate_cu_per_hour: f64,
    /// Sum of known running runtimes.
    pub runtimes: u32,
    /// Accounts with a snapshot.
    pub known: usize,
    /// Accounts without one.
    pub unknown: usize,
    /// Some snapshot is stale.
    pub stale: bool,
}

/// Totals over `views`.
pub fn summarize<'a>(views: impl IntoIterator<Item = &'a AccountUsageView>) -> UsageSummary {
    let mut summary = UsageSummary::default();
    for view in views {
        match view.snapshot {
            Some(snapshot) => {
                summary.balance_cu += snapshot.balance_cu;
                summary.rate_cu_per_hour += snapshot.rate_cu_per_hour;
                summary.runtimes += snapshot.assignments;
                summary.known += 1;
                summary.stale |= snapshot.stale;
            }
            None => summary.unknown += 1,
        }
    }
    summary
}

/// Whether `snapshot` should carry the stale marker at `now`: a later fetch failed, or it is
/// older than core's `USAGE_STALE_AFTER`.
pub fn is_stale(snapshot: &UsageSnapshot, now: Option<SystemTime>) -> bool {
    match now {
        Some(now) => !snapshot.is_fresh(now, USAGE_STALE_AFTER),
        None => snapshot.stale,
    }
}

/// The note that explains a stale `snapshot`: a refresh failed, or (with a slow idle cadence
/// in `Defaults`) the values are only old. Saying a refresh failed when none did would not be
/// honest.
pub fn stale_note(snapshot: &UsageSnapshot) -> &'static str {
    if snapshot.stale {
        keys::USAGE_TIP_STALE
    } else {
        keys::USAGE_TIP_OLD
    }
}

/// Starts the loop that refreshes usage when core says it is due.
pub(crate) fn start_poller(cx: &mut Context<AppState>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_CHECK).await;
            let alive = this.update(cx, |state, cx| state.poll_tick(cx)).is_ok();
            if !alive {
                break;
            }
        }
    })
}

impl AppState {
    /// Usage of every saved account, as last fetched.
    pub fn usage(&self) -> &[AccountUsageView] {
        &self.usage.views
    }

    /// The backend's time when usage was last fetched.
    pub fn usage_now(&self) -> Option<SystemTime> {
        self.usage.now
    }

    /// Why the last refresh failed as a whole (per-account failures are in each view).
    pub fn usage_error(&self) -> Option<&Error> {
        self.usage.error.as_ref()
    }

    /// Usage of one account.
    pub fn usage_for(&self, id: &AccountId) -> Option<&AccountUsageView> {
        self.usage.views.iter().find(|view| &view.account == id)
    }

    /// Usage of the active account.
    pub fn active_usage(&self) -> Option<&AccountUsageView> {
        let id = self.accounts.ready()?.active.as_ref()?;
        self.usage_for(id)
    }

    /// Totals across saved accounts.
    pub fn usage_summary(&self) -> UsageSummary {
        let saved: Vec<AccountId> = self
            .accounts
            .ready()
            .map(|view| view.accounts.iter().map(|a| a.id.clone()).collect())
            .unwrap_or_default();
        let mut summary = summarize(
            self.usage
                .views
                .iter()
                .filter(|view| saved.contains(&view.account)),
        );
        let missing = saved
            .iter()
            .filter(|id| self.usage_for(id).is_none())
            .count();
        summary.unknown += missing;
        summary
    }

    /// Saved accounts with no usage snapshot yet (not loaded, or every fetch failed), by email
    /// (`None` when the account has none): totals do not count them.
    pub fn uncounted_accounts(&self) -> Vec<Option<String>> {
        self.accounts
            .ready()
            .map(|view| {
                view.accounts
                    .iter()
                    .filter(|account| {
                        self.usage_for(&account.id)
                            .is_none_or(|usage| usage.snapshot.is_none())
                    })
                    .map(|account| account.email.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Accounts with runtimes this app does not manage, and how many.
    pub fn unmanaged(&self) -> Vec<(Option<String>, u32)> {
        self.usage
            .views
            .iter()
            .filter(|view| view.unmanaged_runtimes > 0)
            .map(|view| (view.email.clone(), view.unmanaged_runtimes))
            .collect()
    }

    /// Fetches usage: always when `manual`, otherwise only if core says a poll is due.
    pub fn refresh_usage(&mut self, manual: bool, cx: &mut Context<Self>) {
        if !self.cli_installed() {
            return;
        }
        // While the app manages runtimes, each due poll is also a guard round (core's
        // `guard_poll` fetches usage for their accounts and stops what crossed a limit).
        let guard = self.has_managed_runtimes();
        self.run_backend(
            Op::Usage,
            cx,
            move |backend, cancel| {
                if !(manual || backend.usage_poll_due()) {
                    return Ok((None, None, backend.now()));
                }
                let report = guard.then(|| backend.guard_poll(cancel));
                let views = backend.refresh_usage(&[], cancel)?;
                Ok((Some(views), report, backend.now()))
            },
            move |state, result, cx| match result {
                Ok((Some(views), report, now)) => {
                    state.usage.views = views;
                    state.usage.now = Some(now);
                    state.usage.error = None;
                    match report {
                        Some(Ok(report)) => state.apply_guard(report, cx),
                        Some(Err(error)) if error.is(ErrorKind::Busy) => {}
                        Some(Err(error)) => {
                            log::warn!("guard: the round failed: {error}");
                            state.sessions.guard_error = Some(error);
                        }
                        None => {}
                    }
                    state.load_runtimes(cx);
                    cx.notify();
                }
                Ok((None, _, now)) => state.usage.now = Some(now),
                Err(error) if error.is(ErrorKind::Busy) && !manual => {
                    log::info!("usage: the account slot is busy; trying again later");
                }
                Err(error) => {
                    log::warn!("usage: refresh failed: {error}");
                    state.usage.error = Some(error);
                }
            },
        );
    }

    /// One poller wake: refreshes when nothing would be disturbed and core says it is due.
    fn poll_tick(&mut self, cx: &mut Context<Self>) {
        if self.is_provisioning() || self.ops.is_running(Op::PluginRun) {
            // A provisioning progress shows how long Colab has not answered.
            cx.notify();
        }
        // The update check needs no account; its own daily cadence decides whether it asks.
        self.maybe_check_updates(cx);
        let signing_in = matches!(self.dialog, Some(AccountDialog::Add(_)));
        let has_accounts = self
            .accounts
            .ready()
            .is_some_and(|view| !view.accounts.is_empty());
        if signing_in || !has_accounts || self.ops.pausing_poll() {
            return;
        }
        self.refresh_usage(false, cx);
    }
}
