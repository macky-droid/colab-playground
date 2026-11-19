//! The one place that turns core's typed errors into words and a next step.
//!
//! Core never writes prose: an [`Error`] carries an [`ErrorKind`], named parameters, a log
//! detail and a [`NextStep`]. [`explain`] maps the kind to a catalog message (parameters filled
//! leniently, since some are only sometimes known), the next step to either a button the app
//! can act on now ([`ErrorAction`]) or a sentence, and keeps the detail for a "details" line.
//! Kinds and steps added to core later (both enums are non-exhaustive) still show a message
//! naming the kind.

use colab_playground_core::{Error, ErrorKind, NextStep};
use gpui::{App, SharedString};

use crate::i18n::{CATALOGS, Language, fill_lenient, keys, template, tr};

/// Something the app can do about an error right away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorAction {
    Retry,
    /// Check the environment again.
    Refresh,
    /// Go to the CLI install on the Overview page.
    InstallCli,
    /// Open the add-account dialog.
    AddAccount,
    /// Sign in again (the add-account dialog).
    SignInAgain,
    /// Go to the Accounts page to switch.
    SwitchAccount,
    /// Open Microsoft's WSL install guide.
    InstallWsl,
}

impl ErrorAction {
    pub fn label_key(self) -> &'static str {
        match self {
            ErrorAction::Retry => keys::ACTION_RETRY,
            ErrorAction::Refresh => keys::ACTION_CHECK_AGAIN,
            ErrorAction::InstallCli => keys::ACTION_INSTALL_CLI,
            ErrorAction::AddAccount => keys::ACTION_ADD_ACCOUNT,
            ErrorAction::SignInAgain => keys::ACTION_SIGN_IN_AGAIN,
            ErrorAction::SwitchAccount => keys::ACTION_SWITCH_ACCOUNT,
            ErrorAction::InstallWsl => keys::ACTION_INSTALL_WSL,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Explained {
    pub message: SharedString,
    /// What to do, when there is no button for it (or to go with the button).
    pub hint: Option<SharedString>,
    pub action: Option<ErrorAction>,
    /// Technical detail for a "details" line (untranslated, never secret).
    pub detail: Option<SharedString>,
}

/// The catalog key for `kind`, or `None` for a kind this build does not know.
pub fn kind_key(kind: ErrorKind) -> Option<&'static str> {
    use ErrorKind as K;
    Some(match kind {
        K::NotInstalled => keys::ERR_NOT_INSTALLED,
        K::AuthRequired => keys::ERR_AUTH_REQUIRED,
        K::NotEntitled => keys::ERR_NOT_ENTITLED,
        K::CapacityRefused => keys::ERR_CAPACITY_REFUSED,
        K::AllocationTimeout => keys::ERR_ALLOCATION_TIMEOUT,
        K::SshBusy => keys::ERR_SSH_BUSY,
        K::SessionGone => keys::ERR_SESSION_GONE,
        K::ConnectionLost => keys::ERR_CONNECTION_LOST,
        K::ExecTimeout => keys::ERR_EXEC_TIMEOUT,
        K::PortInUse => keys::ERR_PORT_IN_USE,
        K::PortReserved => keys::ERR_PORT_RESERVED,
        K::InvalidInput => keys::ERR_INVALID_INPUT,
        K::NoActiveAccount => keys::ERR_NO_ACTIVE_ACCOUNT,
        K::ActiveAccountNotAdopted => keys::ERR_ACTIVE_NOT_ADOPTED,
        K::AccountNotActive => keys::ERR_ACCOUNT_NOT_ACTIVE,
        K::AccountNotFound => keys::ERR_ACCOUNT_NOT_FOUND,
        K::KeyringUnavailable => keys::ERR_KEYRING_UNAVAILABLE,
        K::AddInProgress => keys::ERR_ADD_IN_PROGRESS,
        K::LegacyTokenPresent => keys::ERR_LEGACY_TOKEN,
        K::LoginCancelled => keys::ERR_LOGIN_CANCELLED,
        K::LoginInvalidCode => keys::ERR_LOGIN_INVALID_CODE,
        K::LoginVerificationFailed => keys::ERR_LOGIN_UNVERIFIED,
        K::Cancelled => keys::ERR_CANCELLED,
        K::Unsupported => keys::ERR_UNSUPPORTED,
        K::WslUnavailable => keys::ERR_WSL_UNAVAILABLE,
        K::CommandTooLong => keys::ERR_COMMAND_TOO_LONG,
        K::SessionLimitReached => keys::ERR_SESSION_LIMIT,
        K::Busy => keys::ERR_BUSY,
        K::ProvisionExhausted => keys::ERR_PROVISION_EXHAUSTED,
        K::OrphanSuspected => keys::ERR_ORPHAN_SUSPECTED,
        K::ManifestInvalid => keys::ERR_MANIFEST_INVALID,
        K::PluginExists => keys::ERR_PLUGIN_EXISTS,
        K::PluginNotFound => keys::ERR_PLUGIN_NOT_FOUND,
        K::PluginRunFailed => keys::ERR_PLUGIN_RUN_FAILED,
        K::HealthCheckFailed => keys::ERR_HEALTH_CHECK_FAILED,
        K::GitFailed => keys::ERR_GIT_FAILED,
        K::InstallFailed => keys::ERR_INSTALL_FAILED,
        K::Http => keys::ERR_HTTP,
        K::HttpUnavailable => keys::ERR_HTTP_UNAVAILABLE,
        K::RateLimited => keys::ERR_RATE_LIMITED,
        K::Parse => keys::ERR_PARSE,
        K::Io => keys::ERR_IO,
        K::Timeout => keys::ERR_TIMEOUT,
        K::UnsupportedCliVersion => keys::ERR_UNSUPPORTED_CLI,
        K::UpdateUnverified => keys::ERR_UPDATE_UNVERIFIED,
        K::UpdateUnavailable => keys::ERR_UPDATE_UNAVAILABLE,
        K::Other => keys::ERR_OTHER,
        _ => return None,
    })
}

/// What `step` offers: a button the app can act on in this build, and/or a sentence.
/// `can_retry` says whether the caller can run the failed thing again.
pub fn next_step(step: NextStep, can_retry: bool) -> (Option<ErrorAction>, Option<&'static str>) {
    use NextStep as S;
    match step {
        S::None => (None, None),
        S::TryAgain if can_retry => (Some(ErrorAction::Retry), None),
        S::TryAgain => (None, Some(keys::NS_TRY_AGAIN)),
        S::RetryLater if can_retry => (Some(ErrorAction::Retry), Some(keys::NS_RETRY_LATER)),
        S::RetryLater => (None, Some(keys::NS_RETRY_LATER)),
        S::SwitchAccount => (Some(ErrorAction::SwitchAccount), None),
        S::AddAccount => (Some(ErrorAction::AddAccount), None),
        S::SignInAgain => (Some(ErrorAction::SignInAgain), None),
        S::UnlockKeyring => (Some(ErrorAction::Refresh), Some(keys::NS_UNLOCK_KEYRING)),
        S::InstallCli => (Some(ErrorAction::InstallCli), None),
        S::InstallWsl => (Some(ErrorAction::InstallWsl), Some(keys::NS_INSTALL_WSL)),
        S::Refresh => (Some(ErrorAction::Refresh), None),
        S::ChooseOtherAccelerator => (None, Some(keys::NS_OTHER_ACCELERATOR)),
        S::StopASession => (None, Some(keys::NS_STOP_A_SESSION)),
        S::ChooseOtherPort => (None, Some(keys::NS_OTHER_PORT)),
        S::CloseOtherConnection => (None, Some(keys::NS_CLOSE_CONNECTION)),
        S::UpdateApp => (None, Some(keys::NS_UPDATE_APP)),
        S::CheckInput => (None, Some(keys::NS_CHECK_INPUT)),
        S::CheckNetwork if can_retry => (Some(ErrorAction::Retry), Some(keys::NS_CHECK_NETWORK)),
        S::CheckNetwork => (None, Some(keys::NS_CHECK_NETWORK)),
        S::ReviewRuntimes => (None, Some(keys::NS_REVIEW_RUNTIMES)),
        S::ViewLog => (None, Some(keys::NS_VIEW_LOG)),
        S::ReportProblem => (None, Some(keys::NS_REPORT_PROBLEM)),
        _ => (None, None),
    }
}

fn has_param(params: &[(&str, &str)], name: &str) -> bool {
    params
        .iter()
        .any(|(key, value)| *key == name && !value.is_empty())
}

// Variants without every parameter fall back to a plain key so raw `{name}` placeholders never show.
fn message_key(kind: ErrorKind, params: &[(&str, &str)]) -> Option<&'static str> {
    match kind {
        ErrorKind::ManifestInvalid => Some(
            if has_param(params, "field") && has_param(params, "reason") {
                keys::ERR_MANIFEST_INVALID
            } else {
                keys::ERR_MANIFEST_INVALID_PLAIN
            },
        ),
        ErrorKind::PluginRunFailed => Some(
            match (has_param(params, "stage"), has_param(params, "message")) {
                (true, true) => keys::ERR_PLUGIN_RUN_FAILED,
                (false, true) => keys::ERR_PLUGIN_RUN_FAILED_MESSAGE,
                (true, false) => keys::ERR_PLUGIN_RUN_FAILED_STAGE,
                (false, false) => keys::ERR_PLUGIN_RUN_FAILED_PLAIN,
            },
        ),
        ErrorKind::InvalidInput => Some(
            match (has_param(params, "field"), has_param(params, "value")) {
                (true, true) => keys::ERR_INVALID_INPUT,
                (true, false) => keys::ERR_INVALID_INPUT_FIELD,
                _ => keys::ERR_INVALID_INPUT_PLAIN,
            },
        ),
        _ => kind_key(kind),
    }
}

fn template_uses_param(raw_template: &str, name: &str) -> bool {
    let mut rest = raw_template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        if &after[..close] == name {
            return true;
        }
        rest = &after[close + 1..];
    }
    false
}

fn explain_with(
    error: &Error,
    can_retry: bool,
    lookup_template: impl Fn(&str) -> &'static str,
) -> Explained {
    let params: Vec<(&str, &str)> = error
        .params
        .iter()
        .filter_map(|(name, value)| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some((name.as_str(), trimmed))
        })
        .collect();
    let (raw_template, message) = match message_key(error.kind, &params) {
        Some(key) => {
            let raw = lookup_template(key);
            (raw, fill_lenient(raw, &params))
        }
        None => {
            let raw = lookup_template(keys::ERR_UNKNOWN);
            (raw, fill_lenient(raw, &[("kind", error.kind.as_str())]))
        }
    };
    // Parameters the chosen message did not name still carry diagnostic context for the details line.
    let unused_params: Vec<String> = params
        .iter()
        .filter(|(name, _)| !template_uses_param(raw_template, name))
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let detail_body = match (error.detail.trim(), unused_params.is_empty()) {
        ("", true) => None,
        ("", false) => Some(unused_params.join(", ")),
        (detail, true) => Some(detail.to_owned()),
        (detail, false) => Some(format!("{} ({})", detail, unused_params.join(", "))),
    };
    let detail = detail_body.map(|body| {
        SharedString::from(fill_lenient(
            lookup_template(keys::ERR_DETAILS),
            &[("detail", body.as_str())],
        ))
    });
    let (action, hint) = next_step(error.next_step, can_retry);
    Explained {
        message: message.into(),
        hint: hint.map(|key| SharedString::from(lookup_template(key))),
        action,
        detail,
    }
}

pub fn explain_in_language(error: &Error, can_retry: bool, language: Language) -> Explained {
    let tag = language.tag();
    let messages = CATALOGS
        .iter()
        .find(|(each, _)| *each == tag)
        .map(|(_, messages)| *messages)
        .unwrap_or(CATALOGS[0].1);
    explain_with(error, can_retry, |key| {
        messages
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, msg)| *msg)
            .unwrap_or_else(|| panic!("i18n: no message {key:?} in {tag}"))
    })
}

pub fn explain(error: &Error, can_retry: bool, cx: &App) -> Explained {
    let mut out = explain_with(error, can_retry, |key| template(cx, key));
    if let (_, Some(key)) = next_step(error.next_step, can_retry) {
        out.hint = Some(tr(cx, key));
    }
    out
}

#[cfg(test)]
mod tests {
    use colab_playground_core::{Error, ErrorKind, NextStep};

    use super::{ErrorAction, explain_in_language, kind_key, next_step};
    use crate::i18n::Language;

    #[test]
    fn every_kind_this_build_knows_has_a_message() {
        let kinds = [
            ErrorKind::NotInstalled,
            ErrorKind::AuthRequired,
            ErrorKind::KeyringUnavailable,
            ErrorKind::LoginInvalidCode,
            ErrorKind::InstallFailed,
            ErrorKind::UnsupportedCliVersion,
            ErrorKind::UpdateUnverified,
            ErrorKind::UpdateUnavailable,
            ErrorKind::Other,
        ];
        for kind in kinds {
            assert!(kind_key(kind).is_some(), "{kind:?}");
        }
    }

    #[test]
    fn steps_become_buttons_only_where_the_app_can_act() {
        assert_eq!(
            next_step(NextStep::InstallCli, false).0,
            Some(ErrorAction::InstallCli)
        );
        assert_eq!(
            next_step(NextStep::TryAgain, true).0,
            Some(ErrorAction::Retry)
        );
        assert_eq!(next_step(NextStep::TryAgain, false).0, None);
        assert!(next_step(NextStep::TryAgain, false).1.is_some());
        assert_eq!(next_step(NextStep::StopASession, true).0, None);
        assert_eq!(next_step(NextStep::None, true), (None, None));
    }

    #[test]
    fn manifest_plugin_and_input_errors_interpolate_details_and_fall_back_cleanly() {
        let manifest_err = Error::new(ErrorKind::ManifestInvalid)
            .with("field", "api")
            .with("reason", "unsupported api version");
        let run_err = Error::new(ErrorKind::PluginRunFailed)
            .with("stage", "download")
            .with("message", "HTTP 401");
        let input_err = Error::new(ErrorKind::InvalidInput)
            .with("field", "port")
            .with("value", "abc");

        for lang in Language::ALL {
            let m = explain_in_language(&manifest_err, false, lang);
            assert!(
                m.message.contains("api") && m.message.contains("unsupported api version"),
                "{lang:?}: {}",
                m.message
            );
            assert!(!m.message.contains('{'), "{lang:?}: {}", m.message);

            let r = explain_in_language(&run_err, false, lang);
            assert!(
                r.message.contains("download") && r.message.contains("HTTP 401"),
                "{lang:?}: {}",
                r.message
            );
            assert!(!r.message.contains('{'), "{lang:?}: {}", r.message);

            let i = explain_in_language(&input_err, false, lang);
            assert!(
                i.message.contains("port") && i.message.contains("abc"),
                "{lang:?}: {}",
                i.message
            );
            assert!(!i.message.contains('{'), "{lang:?}: {}", i.message);
        }

        let crash_only_msg = Error::new(ErrorKind::PluginRunFailed)
            .with("stage", "   ")
            .with("message", "server exited (code 1)");
        let explained = explain_in_language(&crash_only_msg, false, Language::EnUs);
        assert!(explained.message.contains("server exited (code 1)"));
        assert!(!explained.message.contains('{'));

        let field_only_input = Error::invalid("repo", "expected owner/name");
        let explained = explain_in_language(&field_only_input, false, Language::EnUs);
        assert!(explained.message.contains("repo"));
        assert!(!explained.message.contains('{'));
        assert!(
            explained
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("expected owner/name"))
        );

        let partial_manifest = Error::new(ErrorKind::ManifestInvalid).with("field", "run.script");
        let explained = explain_in_language(&partial_manifest, false, Language::EnUs);
        assert!(!explained.message.contains('{'));
        assert!(
            explained
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("field=run.script")),
            "{explained:?}"
        );
    }
}
