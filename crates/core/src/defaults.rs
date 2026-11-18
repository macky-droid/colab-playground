//! User-editable defaults: no policy value is hard-coded (an invariant in `docs/DESIGN.md`).
//!
//! Every default policy value lives in [`Defaults`]. The app persists it in its
//! settings file and pre-fills the new-session dialog and plugin run forms
//! from it; each value can be overridden for a single action. Only hard
//! safety caps (such as [`crate::provision::MAX_ATTEMPTS_CAP`]) stay constants.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::clock::serde_dur;
use crate::guard::GuardPolicy;
use crate::provision::{OnExhausted, ProvisionSpec, SessionLimit};
use crate::sessions::Accelerator;

/// Default policy values. Missing fields in stored JSON take their default,
/// so older settings files keep loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    /// Runtime limit for new sessions (VM watchdog deadline). Default 2 h.
    #[serde(with = "serde_dur::option")]
    pub max_runtime: Option<Duration>,
    /// Spend cap per session in compute units. Default none.
    pub max_spend_cu: Option<f64>,
    /// Stop all sessions of an account below this balance. Default none.
    pub min_balance_cu: Option<f64>,
    /// VM idle rule. Default none.
    #[serde(with = "serde_dur::option")]
    pub idle_timeout: Option<Duration>,
    /// Sessions the app keeps per account at most. Default 2.
    pub max_concurrent_sessions: u32,
    /// Provisioning attempts. Default 3 (always clamped to the hard cap of 8).
    pub max_attempts: u32,
    /// Wait between provisioning attempts. Default 20 s.
    #[serde(with = "serde_dur")]
    pub retry_delay: Duration,
    /// What to do when no attempt met the spec. Default stop and fail.
    pub on_exhausted: OnExhausted,
    /// Usage poll interval while the account has sessions. Default 30 s.
    #[serde(with = "serde_dur")]
    pub usage_poll_active: Duration,
    /// Usage poll interval otherwise. Default 5 min.
    #[serde(with = "serde_dur")]
    pub usage_poll_idle: Duration,
    /// Accelerator pre-selected for new sessions. Default A100.
    pub accelerator: Accelerator,
    /// Request the high-RAM shape. Default false.
    pub high_mem: bool,
    /// Minimum GPU memory for new sessions. Default none.
    pub min_vram_mib: Option<u32>,
    /// The VM watchdog's wind-down phase starts this long before the deadline
    /// (time for data-keeping hooks). Default 300 s.
    #[serde(with = "serde_dur")]
    pub winddown_grace: Duration,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            max_runtime: Some(Duration::from_secs(2 * 3600)),
            max_spend_cu: None,
            min_balance_cu: None,
            idle_timeout: None,
            max_concurrent_sessions: 2,
            max_attempts: 3,
            retry_delay: Duration::from_secs(20),
            on_exhausted: OnExhausted::StopAndFail,
            usage_poll_active: Duration::from_secs(30),
            usage_poll_idle: Duration::from_secs(5 * 60),
            accelerator: Accelerator::A100,
            high_mem: false,
            min_vram_mib: None,
            winddown_grace: Duration::from_secs(300),
        }
    }
}

impl Defaults {
    pub fn guard_policy(&self) -> GuardPolicy {
        GuardPolicy {
            max_runtime: self.max_runtime,
            max_spend_cu: self.max_spend_cu,
            min_balance_cu: self.min_balance_cu,
            idle_timeout: self.idle_timeout,
        }
    }

    pub fn provision_spec(&self, name: impl Into<String>) -> ProvisionSpec {
        ProvisionSpec {
            name: name.into(),
            accelerator: self.accelerator,
            high_mem: self.high_mem,
            min_vram_mib: self.min_vram_mib,
            max_attempts: self.max_attempts,
            retry_delay: self.retry_delay,
            on_exhausted: self.on_exhausted,
            guard: self.guard_policy(),
        }
    }

    pub fn session_limit(&self) -> SessionLimit {
        SessionLimit {
            max_concurrent: self.max_concurrent_sessions,
            reserved: 0,
        }
    }

    pub fn usage_poll_interval(&self, has_sessions: bool) -> Duration {
        if has_sessions {
            self.usage_poll_active
        } else {
            self.usage_poll_idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_defaults() {
        let d = Defaults::default();
        assert_eq!(d.max_runtime, Some(Duration::from_secs(7200)));
        assert_eq!(d.max_concurrent_sessions, 2);
        assert_eq!(d.max_attempts, 3);
        assert_eq!(d.retry_delay, Duration::from_secs(20));
        assert_eq!(d.on_exhausted, OnExhausted::StopAndFail);
        assert_eq!(d.usage_poll_interval(true), Duration::from_secs(30));
        assert_eq!(d.usage_poll_interval(false), Duration::from_secs(300));
        assert_eq!(d.max_spend_cu, None);
    }

    #[test]
    fn old_files_keep_loading_and_values_round_trip() {
        let d: Defaults = serde_json::from_str(r#"{"max_attempts": 5}"#).unwrap();
        assert_eq!(d.max_attempts, 5);
        assert_eq!(d.max_concurrent_sessions, 2);
        let d: Defaults =
            serde_json::from_str(r#"{"max_runtime": null, "unknown_future_field": 1}"#).unwrap();
        assert_eq!(d.max_runtime, None);
        let custom = Defaults {
            idle_timeout: Some(Duration::from_secs(900)),
            ..Defaults::default()
        };
        let back: Defaults =
            serde_json::from_str(&serde_json::to_string(&custom).unwrap()).unwrap();
        assert_eq!(back, custom);
    }

    #[test]
    fn builders_use_the_defaults() {
        let d = Defaults {
            max_attempts: 4,
            accelerator: Accelerator::L4,
            ..Defaults::default()
        };
        let spec = d.provision_spec("s1");
        assert_eq!(spec.max_attempts, 4);
        assert_eq!(spec.accelerator, Accelerator::L4);
        assert_eq!(spec.guard.max_runtime, d.max_runtime);
        assert_eq!(d.session_limit().max_concurrent, 2);
    }
}
