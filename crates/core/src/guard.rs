//! App-side cost guard.
//!
//! The VM-side watchdog (see [`crate::watchdog`]) is the safety net that
//! survives the app; this module is the second layer that runs while the app
//! is open. Every usage poll charges each session's [`SpendMeter`] with the
//! session's *estimated* rate (see [`crate::usage::RateAttribution`]) and then
//! calls the pure [`evaluate`] function.
//!
//! Rules (all thresholds are inclusive):
//!
//! * **Runtime** — `elapsed >= max_runtime` stops; `>= 80 %` warns.
//! * **Spend** — `spend >= max_spend_cu` stops; `>= 80 %` warns. Spend is the
//!   integral of the session's estimated rate over time. Poll gaps are filled
//!   with the last known rate (marked `estimated`), never with zero. If no rate
//!   was ever observed the spend is unknown and the guard warns instead of
//!   assuming it is zero.
//! * **Balance floor** — the balance projected to `now` (last balance minus
//!   burn since that sample) `<= min_balance_cu` stops; within 25 % above the
//!   floor (i.e. the floor is at least 80 % of the balance) warns. Missing,
//!   stale (a later fetch failed) or old (> 5 min) usage warns.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::clock::{serde_dur, serde_secs};
use crate::usage::UsageSnapshot;

pub const WARN_FRACTION: f64 = 0.8;

/// Usage older than this is considered stale for the balance floor.
pub const USAGE_STALE_AFTER: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardPolicy {
    /// Absolute runtime limit (VM watchdog deadline and app-side stop).
    #[serde(default, with = "serde_dur::option")]
    pub max_runtime: Option<Duration>,
    #[serde(default)]
    pub max_spend_cu: Option<f64>,
    /// Stop every session of the account when its balance falls to this.
    #[serde(default)]
    pub min_balance_cu: Option<f64>,
    /// VM-side idle rule: no plugin heartbeat and an idle GPU for this long.
    #[serde(default, with = "serde_dur::option")]
    pub idle_timeout: Option<Duration>,
}

impl Default for GuardPolicy {
    fn default() -> Self {
        crate::defaults::Defaults::default().guard_policy()
    }
}

impl GuardPolicy {
    pub fn unlimited() -> Self {
        GuardPolicy {
            max_runtime: None,
            max_spend_cu: None,
            min_balance_cu: None,
            idle_timeout: None,
        }
    }

    /// True when the VM-side watchdog has something to enforce.
    pub fn needs_watchdog(&self) -> bool {
        self.max_runtime.is_some() || self.idle_timeout.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpendMeter {
    /// When the session started (integration origin).
    #[serde(with = "serde_secs")]
    pub started_at: SystemTime,
    /// Compute units attributed so far (meaningful only if `rate_known`).
    pub spend_cu: f64,
    /// Whether any burn rate has ever been observed for this session.
    pub rate_known: bool,
    /// True if part of `spend_cu` was extrapolated over poll gaps.
    pub estimated: bool,
    #[serde(with = "serde_secs")]
    pub integrated_to: SystemTime,
    /// Last observed per-session rate (CU/h), used to fill gaps.
    pub last_share_cu_per_hour: Option<f64>,
    /// When usage was last successfully observed.
    #[serde(default, with = "serde_secs::option")]
    pub last_observed: Option<SystemTime>,
}

impl SpendMeter {
    pub fn new(started_at: SystemTime) -> Self {
        SpendMeter {
            started_at,
            spend_cu: 0.0,
            rate_known: false,
            estimated: false,
            integrated_to: started_at,
            last_share_cu_per_hour: None,
            last_observed: None,
        }
    }

    /// Charges the interval since the last record at `rate_cu_per_hour` (this
    /// session's estimated rate). `None` means this reading has no rate for the
    /// session (the poll failed, or its share is not known yet): the interval
    /// is charged at the last known rate and the meter becomes `estimated`;
    /// with no rate known at all the gap stays open (and `rate_known` false) so
    /// a later reading charges it — callers must treat that as unknown, never
    /// as zero spend.
    pub fn record(&mut self, now: SystemTime, rate_cu_per_hour: Option<f64>) {
        let dt_h = now
            .duration_since(self.integrated_to)
            .unwrap_or_default()
            .as_secs_f64()
            / 3600.0;
        match (
            rate_cu_per_hour.map(|r| r.max(0.0)),
            self.last_share_cu_per_hour,
        ) {
            (Some(r), _) => {
                self.spend_cu += r * dt_h;
                self.rate_known = true;
                self.last_share_cu_per_hour = Some(r);
                self.last_observed = Some(now);
                self.integrated_to = now;
            }
            (None, Some(prev)) => {
                self.spend_cu += prev * dt_h;
                self.estimated = true;
                self.integrated_to = now;
            }
            (None, None) => {}
        }
    }

    pub fn spend(&self) -> Option<f64> {
        self.rate_known.then_some(self.spend_cu)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    /// `max_runtime` (values in seconds).
    Runtime,
    /// `max_spend_cu` (values in compute units).
    Spend,
    /// `min_balance_cu` (values in compute units; lower is worse).
    Balance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Concern {
    Limit {
        limit: Limit,
        /// Current value (seconds or compute units).
        current: f64,
        threshold: f64,
        /// Spend includes extrapolated intervals.
        estimated: bool,
    },
    /// Usage could not be determined, so a limit cannot be verified.
    UsageUnknown {
        /// The limit that cannot be checked.
        limit: Limit,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", content = "concerns", rename_all = "snake_case")]
pub enum Decision {
    Ok,
    /// At least one limit is at 80 % or usage is unknown.
    Warn(Vec<Concern>),
    /// A limit is reached: stop the session.
    Stop(Concern),
}

/// Pure: depends only on its arguments.
pub fn evaluate(
    policy: &GuardPolicy,
    now: SystemTime,
    usage: Option<&UsageSnapshot>,
    meter: &SpendMeter,
) -> Decision {
    let mut warns = Vec::new();

    if let Some(max) = policy.max_runtime {
        let elapsed = now
            .duration_since(meter.started_at)
            .unwrap_or_default()
            .as_secs_f64();
        let max = max.as_secs_f64();
        let c = Concern::Limit {
            limit: Limit::Runtime,
            current: elapsed,
            threshold: max,
            estimated: false,
        };
        if elapsed >= max {
            return Decision::Stop(c);
        }
        if elapsed >= max * WARN_FRACTION {
            warns.push(c);
        }
    }

    if let Some(max) = policy.max_spend_cu {
        match meter.spend() {
            Some(spend) => {
                let c = Concern::Limit {
                    limit: Limit::Spend,
                    current: spend,
                    threshold: max,
                    estimated: meter.estimated,
                };
                if spend >= max {
                    return Decision::Stop(c);
                }
                if spend >= max * WARN_FRACTION {
                    warns.push(c);
                }
            }
            None => warns.push(Concern::UsageUnknown {
                limit: Limit::Spend,
            }),
        }
    }

    if let Some(floor) = policy.min_balance_cu {
        match usage {
            Some(sample) if sample.is_fresh(now, USAGE_STALE_AFTER) => {
                let balance = sample.projected_balance(now);
                let c = Concern::Limit {
                    limit: Limit::Balance,
                    current: balance,
                    threshold: floor,
                    estimated: false,
                };
                if balance <= floor {
                    return Decision::Stop(c);
                }
                if floor >= balance * WARN_FRACTION {
                    warns.push(c);
                }
            }
            _ => warns.push(Concern::UsageUnknown {
                limit: Limit::Balance,
            }),
        }
    }

    if warns.is_empty() {
        Decision::Ok
    } else {
        Decision::Warn(warns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }
    fn mins(m: u64) -> Duration {
        Duration::from_secs(m * 60)
    }
    fn sample(at: SystemTime, balance: f64, rate: f64) -> UsageSnapshot {
        UsageSnapshot::new(
            crate::usage::Usage {
                balance_cu: balance,
                rate_cu_per_hour: rate,
                assignments: 1,
            },
            at,
        )
    }
    fn runtime_only(m: u64) -> GuardPolicy {
        GuardPolicy {
            max_runtime: Some(mins(m)),
            ..GuardPolicy::unlimited()
        }
    }

    #[test]
    fn default_policy_is_two_hours() {
        assert_eq!(
            GuardPolicy::default().max_runtime,
            Some(Duration::from_secs(7200))
        );
        assert!(GuardPolicy::default().needs_watchdog());
        assert!(!GuardPolicy::unlimited().needs_watchdog());
    }

    #[test]
    fn deadline_ok_warn_stop() {
        let p = runtime_only(100);
        let m = SpendMeter::new(t0());
        assert_eq!(evaluate(&p, t0() + mins(79), None, &m), Decision::Ok);
        match evaluate(&p, t0() + mins(80), None, &m) {
            Decision::Warn(c) => assert!(matches!(
                c[0],
                Concern::Limit {
                    limit: Limit::Runtime,
                    ..
                }
            )),
            d => panic!("{d:?}"),
        }
        assert!(matches!(
            evaluate(&p, t0() + mins(100), None, &m),
            Decision::Stop(_)
        ));
        assert!(matches!(
            evaluate(&p, t0() + mins(500), None, &m),
            Decision::Stop(_)
        ));
    }

    #[test]
    fn spend_cap_integrates_rate() {
        let p = GuardPolicy {
            max_spend_cu: Some(10.0),
            ..GuardPolicy::unlimited()
        };
        let mut m = SpendMeter::new(t0());
        // 5 CU/h for one hour = 5 CU: fine.
        m.record(t0() + mins(60), Some(5.0));
        assert!((m.spend_cu - 5.0).abs() < 1e-9);
        assert_eq!(evaluate(&p, t0() + mins(60), None, &m), Decision::Ok);
        // Another 36 min at 5 CU/h = 8 CU total: warn.
        m.record(t0() + mins(96), Some(5.0));
        assert!(matches!(
            evaluate(&p, t0() + mins(96), None, &m),
            Decision::Warn(_)
        ));
        // 2 CU more reaches the cap.
        m.record(t0() + mins(120), Some(5.0));
        assert!(matches!(
            evaluate(&p, t0() + mins(120), None, &m),
            Decision::Stop(_)
        ));
    }

    #[test]
    fn unknown_usage_is_never_zero() {
        let p = GuardPolicy {
            max_spend_cu: Some(1.0),
            ..GuardPolicy::unlimited()
        };
        let mut m = SpendMeter::new(t0());
        m.record(t0() + mins(30), None);
        assert_eq!(m.spend(), None);
        assert_eq!(
            evaluate(&p, t0() + mins(30), None, &m),
            Decision::Warn(vec![Concern::UsageUnknown {
                limit: Limit::Spend
            }])
        );
        // The gap stays open: the first real reading charges the whole hour.
        m.record(t0() + mins(60), Some(2.0));
        assert!((m.spend_cu - 2.0).abs() < 1e-9);
        assert!(matches!(
            evaluate(&p, t0() + mins(60), None, &m),
            Decision::Stop(_)
        ));
    }

    #[test]
    fn poll_gaps_are_extrapolated_and_marked() {
        let mut m = SpendMeter::new(t0());
        m.record(t0() + mins(60), Some(4.0));
        m.record(t0() + mins(90), None);
        assert!((m.spend_cu - 6.0).abs() < 1e-9);
        assert!(m.estimated);
        let p = GuardPolicy {
            max_spend_cu: Some(6.0),
            ..GuardPolicy::unlimited()
        };
        match evaluate(&p, t0() + mins(90), None, &m) {
            Decision::Stop(Concern::Limit { estimated, .. }) => assert!(estimated),
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn balance_floor_stop_warn_and_projection() {
        let p = GuardPolicy {
            min_balance_cu: Some(10.0),
            ..GuardPolicy::unlimited()
        };
        let m = SpendMeter::new(t0());
        let now = t0() + mins(10);
        assert_eq!(
            evaluate(&p, now, Some(&sample(now, 50.0, 1.0)), &m),
            Decision::Ok
        );
        // Floor is >= 80 % of the balance: warn.
        assert!(matches!(
            evaluate(&p, now, Some(&sample(now, 12.5, 1.0)), &m),
            Decision::Warn(_)
        ));
        assert!(matches!(
            evaluate(&p, now, Some(&sample(now, 10.0, 1.0)), &m),
            Decision::Stop(_)
        ));
        // 4 minutes old at 60 CU/h: 13 - 4 = 9 projected -> stop.
        let older = sample(now - mins(4), 13.0, 60.0);
        assert!(matches!(
            evaluate(&p, now, Some(&older), &m),
            Decision::Stop(_)
        ));
    }

    #[test]
    fn missing_or_stale_usage_warns_for_balance() {
        let p = GuardPolicy {
            min_balance_cu: Some(10.0),
            ..GuardPolicy::unlimited()
        };
        let m = SpendMeter::new(t0());
        let now = t0() + mins(30);
        let unknown = Decision::Warn(vec![Concern::UsageUnknown {
            limit: Limit::Balance,
        }]);
        assert_eq!(evaluate(&p, now, None, &m), unknown);
        let old = sample(now - mins(6), 500.0, 0.0);
        assert_eq!(evaluate(&p, now, Some(&old), &m), unknown);
        let mut stale = sample(now, 500.0, 0.0);
        stale.stale = true;
        assert_eq!(evaluate(&p, now, Some(&stale), &m), unknown);
    }

    #[test]
    fn stop_wins_over_warnings_and_warnings_accumulate() {
        let p = GuardPolicy {
            max_runtime: Some(mins(100)),
            max_spend_cu: Some(10.0),
            min_balance_cu: Some(10.0),
            idle_timeout: None,
        };
        let mut m = SpendMeter::new(t0());
        m.record(t0() + mins(85), Some(6.0));
        let now = t0() + mins(85);
        match evaluate(&p, now, Some(&sample(now, 12.0, 6.0)), &m) {
            Decision::Warn(c) => assert_eq!(c.len(), 3),
            d => panic!("{d:?}"),
        }
        let late = t0() + mins(100);
        assert!(matches!(
            evaluate(&p, late, Some(&sample(late, 12.0, 6.0)), &m),
            Decision::Stop(Concern::Limit {
                limit: Limit::Runtime,
                ..
            })
        ));
    }
}
