//! Asking for a star or feedback, once the app has proved useful.
//!
//! The ask waits until [`READY_BEFORE_ASKING`] runtimes or deployments have come up, then comes
//! at a pause: just after the user stopped a runtime, with nothing else billing or starting and
//! no other dialog open. It comes at most [`MOST_ASKS`] times, each no sooner than
//! [`ASK_AGAIN_AFTER`] after the one before, and never again once the user opened either link
//! from it. Both links only open the browser; the app sends nothing.

use std::time::Duration;

use gpui::Context;

use super::{AppState, Op, Overlay, updates::secs};
use crate::{platform, settings::Settings};

/// Runtimes and deployments that must have come up before the app asks.
pub const READY_BEFORE_ASKING: u32 = 3;
pub const MOST_ASKS: u32 = 2;
/// How long "Not now" puts the next ask off.
pub const ASK_AGAIN_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);

/// The project's page on GitHub (the package repository), where it can be starred.
pub const PROJECT_PAGE: &str = env!("CARGO_PKG_REPOSITORY");

/// GitHub's form for a new issue, with this build's version and system filled in under the
/// place where the user writes.
pub fn feedback_page() -> String {
    let build = format!(
        "\n\n{} {} ({})",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        platform::system()
    );
    let body: String = url::form_urlencoded::byte_serialize(build.as_bytes()).collect();
    format!("{PROJECT_PAGE}/issues/new?body={body}")
}

/// Whether the app may ask at `now` (seconds since the Unix epoch).
fn due(settings: &Settings, now: u64) -> bool {
    !settings.feedback_answered
        && settings.ready_count >= READY_BEFORE_ASKING
        && settings.feedback_asked < MOST_ASKS
        && settings
            .feedback_asked_at
            .is_none_or(|at| now >= at.saturating_add(ASK_AGAIN_AFTER.as_secs()))
}

impl AppState {
    /// A runtime or deployment came up.
    pub(crate) fn count_ready(&mut self, cx: &mut Context<Self>) {
        self.settings.ready_count = self.settings.ready_count.saturating_add(1);
        self.persist_settings(cx);
    }

    /// The user's stop ended a runtime. The list shown can still lack a runtime that came up
    /// meanwhile (a start that finished during the stop, one opened in the browser), and that
    /// one bills, so the ask waits for the listing under way.
    pub(crate) fn ask_for_feedback_after_stop(&mut self, cx: &mut Context<Self>) {
        if self.ops.is_running(Op::Runtimes) {
            self.feedback_after_listing = true;
        } else {
            self.ask_for_feedback_if_due(cx);
        }
    }

    /// A runtime listing is back: a stop waiting for it asks now, if it may.
    pub(crate) fn listed_for_feedback(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.feedback_after_listing) {
            self.ask_for_feedback_if_due(cx);
        }
    }

    /// Asks for a star or feedback when it is due and nothing else needs the user.
    fn ask_for_feedback_if_due(&mut self, cx: &mut Context<Self>) {
        let Some(now) = self.backend_now().map(secs) else {
            return;
        };
        let busy = self.overlay.is_some() || self.dialog.is_some() || self.billing().any();
        if busy || !due(&self.settings, now) {
            return;
        }
        self.settings.feedback_asked += 1;
        self.settings.feedback_asked_at = Some(now);
        log::info!(
            "feedback: asking for a star or feedback ({} of {MOST_ASKS})",
            self.settings.feedback_asked
        );
        self.persist_settings(cx);
        self.overlay = Some(Overlay::Feedback);
        cx.notify();
    }

    /// Whether the app will not ask again after this time, so dismissing can say so.
    pub fn feedback_asked_for_the_last_time(&self) -> bool {
        self.settings.feedback_asked >= MOST_ASKS
    }

    /// Opens the project page from the prompt: GitHub has no link that stars a repository, so
    /// the user stars it there.
    pub fn star_from_prompt(&mut self, cx: &mut Context<Self>) {
        self.answer_prompt(PROJECT_PAGE, cx);
    }

    /// Opens a new issue from the prompt.
    pub fn give_feedback_from_prompt(&mut self, cx: &mut Context<Self>) {
        self.answer_prompt(&feedback_page(), cx);
    }

    fn answer_prompt(&mut self, url: &str, cx: &mut Context<Self>) {
        cx.open_url(url);
        self.settings.feedback_answered = true;
        self.persist_settings(cx);
        self.close_overlay(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ASK_AGAIN_AFTER, MOST_ASKS, PROJECT_PAGE, READY_BEFORE_ASKING, due, feedback_page,
    };
    use crate::settings::Settings;

    fn used(ready_count: u32) -> Settings {
        Settings {
            ready_count,
            ..Settings::default()
        }
    }

    #[test]
    fn it_waits_until_a_few_runtimes_came_up() {
        assert!(!due(&used(READY_BEFORE_ASKING - 1), 0));
        assert!(due(&used(READY_BEFORE_ASKING), 0));
    }

    #[test]
    fn not_now_puts_the_next_ask_off_and_the_asks_end() {
        let asked = 1_767_225_600;
        let later = ASK_AGAIN_AFTER.as_secs();
        let mut settings = Settings {
            feedback_asked: 1,
            feedback_asked_at: Some(asked),
            ..used(READY_BEFORE_ASKING)
        };
        assert!(!due(&settings, asked + later - 1));
        assert!(due(&settings, asked + later));
        settings.feedback_asked = MOST_ASKS;
        assert!(!due(&settings, asked + later * 10));
    }

    #[test]
    fn an_answer_ends_the_asks() {
        let settings = Settings {
            feedback_answered: true,
            ..used(READY_BEFORE_ASKING)
        };
        assert!(!due(&settings, 0));
    }

    #[test]
    fn the_issue_form_carries_the_version_and_the_system() {
        let page = feedback_page();
        let query = page
            .strip_prefix(&format!("{PROJECT_PAGE}/issues/new?"))
            .expect("the project's issue form");
        let body: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let [(name, text)] = body.as_slice() else {
            panic!("one body parameter: {body:?}");
        };
        assert_eq!(name, "body");
        assert!(text.starts_with("\n\n"), "room to write above: {text:?}");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text:?}");
        assert!(text.contains(&crate::platform::system()), "{text:?}");
    }
}
