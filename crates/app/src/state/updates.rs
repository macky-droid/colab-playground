//! The update check: whether a newer release is published, never downloading or installing.
//!
//! Automatic checks run from the usage poller (so never before the first frame or the setup
//! checks), at most once a day on the backend clock; the time of the last check is saved, so a
//! restart within the day does not check again. A failed automatic check is only shown in
//! Settings → About; a manual one also says so in a toast. A found release is announced once
//! per version (a banner the user can dismiss for that version) and stays marked in the sidebar
//! and in About until this build is at least that version.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use colab_playground_core::{
    Error,
    update::{self, Release, UpdateStatus, Version},
};
use ely_gpui_component::{feedback::Toast, primitives::Severity};
use gpui::Context;

use super::{AppState, Op};
use crate::{
    errors,
    i18n::{keys, tr},
    settings::OnByDefault,
};

/// How often the automatic check asks GitHub.
pub const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateCheck {
    /// Not checked in this run.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Failed(Error),
}

fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn is_newer(version: &str) -> bool {
    match (
        Version::parse(version),
        Version::parse(update::CURRENT_VERSION),
    ) {
        (Some(found), Some(current)) => found > current,
        _ => false,
    }
}

impl AppState {
    pub fn update_check(&self) -> &UpdateCheck {
        &self.update
    }

    /// The newer release to point at (found now or by an earlier check), while this build is
    /// older than it: `(version, page)`.
    pub fn update_available(&self) -> Option<(String, String)> {
        if let UpdateCheck::Available(release) = &self.update {
            return Some((release.version.clone(), release.page.clone()));
        }
        let version = self.settings.update_latest.clone()?;
        if !is_newer(&version) {
            return None;
        }
        let page = update::this_app_release_page(&version)?;
        Some((version, page))
    }

    pub fn update_notice(&self) -> Option<(String, String)> {
        self.update_available()
            .filter(|(version, _)| self.settings.update_dismissed.as_deref() != Some(version))
    }

    pub fn checks_updates(&self) -> bool {
        self.settings.check_updates.0
    }

    pub fn set_check_updates(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.check_updates = OnByDefault(on);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Hides the banner for `version`; the sidebar and About keep pointing at it.
    pub fn dismiss_update(&mut self, version: String, cx: &mut Context<Self>) {
        self.settings.update_dismissed = Some(version);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Opens the release page in the browser (the app never downloads anything itself).
    pub fn open_update_page(&self, cx: &mut Context<Self>) {
        if let Some((_, page)) = self.update_available() {
            cx.open_url(&page);
        }
    }

    pub fn check_updates_now(&mut self, cx: &mut Context<Self>) {
        self.run_update_check(true, cx);
    }

    /// One poller wake: checks when automatic checks are on and a day has passed since the
    /// last check (on the backend clock).
    pub(crate) fn maybe_check_updates(&mut self, cx: &mut Context<Self>) {
        if self.checks_updates() && !self.ops.is_running(Op::UpdateCheck) {
            self.run_update_check(false, cx);
        }
    }

    fn run_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.ops.is_running(Op::UpdateCheck) {
            return;
        }
        let last = self.settings.update_checked_at;
        if manual {
            self.update = UpdateCheck::Checking;
        }
        self.run_backend(
            Op::UpdateCheck,
            cx,
            move |backend, cancel| {
                let now = secs(backend.now());
                let due = manual || last.is_none_or(|last| now >= last + UPDATE_INTERVAL.as_secs());
                if !due {
                    return Ok(None);
                }
                Ok(Some((backend.check_for_update(cancel), now)))
            },
            move |state, result, cx| {
                let Ok(Some((outcome, now))) = result else {
                    if let Err(error) = result {
                        state.update_failed(error, manual, cx);
                    }
                    return;
                };
                state.settings.update_checked_at = Some(now);
                match outcome {
                    Ok(UpdateStatus::UpToDate) => {
                        state.update = UpdateCheck::UpToDate;
                        if state
                            .settings
                            .update_latest
                            .as_deref()
                            .is_some_and(|v| !is_newer(v))
                        {
                            state.settings.update_latest = None;
                        }
                    }
                    Ok(UpdateStatus::Available(release)) => {
                        state.settings.update_latest = Some(release.version.clone());
                        state.update = UpdateCheck::Available(release);
                    }
                    Err(error) => state.update_failed(error, manual, cx),
                }
                state.persist_settings(cx);
                cx.notify();
            },
        );
        cx.notify();
    }

    fn update_failed(&mut self, error: Error, manual: bool, cx: &mut Context<Self>) {
        log::warn!("updates: the check failed: {error}");
        if manual {
            let message = errors::explain(&error, false, cx).message;
            let toast = Toast::new(tr(cx, keys::UPDATE_FAILED_TITLE))
                .body(message)
                .severity(Severity::Warning);
            self.toast(toast, cx);
        }
        self.update = UpdateCheck::Failed(error);
        cx.notify();
    }
}
