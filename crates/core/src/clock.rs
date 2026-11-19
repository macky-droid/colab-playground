//! Injected time and cooperative cancellation.
//!
//! Retry delays, polling intervals and guard decisions all read time through a
//! [`Clock`], so tests and the mock backend can run them instantly with a
//! [`ManualClock`] while the real app uses [`SystemClock`].

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::error::{Error, Result};

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;

    /// Returns `Err(Cancelled)` if the token fired before the time elapsed.
    /// Implementations must never block forever: [`ManualClock`] advances its
    /// own time and returns immediately.
    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<()>;
}

/// `sleep` waits in short slices so cancellation is prompt.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<()> {
        let end = Instant::now() + duration;
        loop {
            if cancel.is_cancelled() {
                return Err(Error::cancelled());
            }
            let now = Instant::now();
            if now >= end {
                return Ok(());
            }
            std::thread::sleep((end - now).min(Duration::from_millis(50)));
        }
    }
}

/// A deterministic clock for tests and the headless mock.
///
/// `sleep` advances the clock by the requested duration and returns at once,
/// so code that "waits 20 s between retries" completes instantly while still
/// observing the time it would have taken. [`ManualClock::advance`] moves time
/// forward from the outside (e.g. a UI test fast-forwarding an hour of usage).
#[derive(Debug, Clone)]
pub struct ManualClock {
    now: Arc<Mutex<SystemTime>>,
}

impl ManualClock {
    pub fn new(start: SystemTime) -> Self {
        ManualClock {
            now: Arc::new(Mutex::new(start)),
        }
    }

    /// A clock starting at a fixed, arbitrary instant (2026-01-01T00:00:00Z),
    /// so test output is reproducible.
    pub fn fixed() -> Self {
        ManualClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_225_600))
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        *now += by;
    }

    pub fn set(&self, to: SystemTime) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) = to;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        ManualClock::fixed()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(Error::cancelled());
        }
        self.advance(duration);
        Ok(())
    }
}

/// Cloning shares the state. Work with a point of no return [commits](CancelToken::commit)
/// there, and a cancel after that no longer applies.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicU8>);

const LIVE: u8 = 0;
const CANCELLED: u8 = 1;
const COMMITTED: u8 = 2;

impl CancelToken {
    pub fn new() -> Self {
        CancelToken::default()
    }

    /// Idempotent. False when the work has already committed: it runs to its end.
    pub fn cancel(&self) -> bool {
        match self
            .0
            .compare_exchange(LIVE, CANCELLED, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => true,
            Err(state) => state == CANCELLED,
        }
    }

    /// True once [`CancelToken::cancel`] has taken effect on any clone.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CANCELLED
    }

    /// The work's point of no return: from here on it finishes whatever a cancel says, and the
    /// caller of [`CancelToken::cancel`] learns that it did not apply. Fails as cancelled when a
    /// cancel came first. Deciding both in one step leaves no moment in which a cancel is
    /// accepted while the work carries on.
    pub fn commit(&self) -> Result<()> {
        match self
            .0
            .compare_exchange(LIVE, COMMITTED, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => Ok(()),
            Err(COMMITTED) => Ok(()),
            Err(_) => Err(Error::cancelled()),
        }
    }

    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::cancelled())
        } else {
            Ok(())
        }
    }
}

/// Seconds since the Unix epoch (saturating at 0 for earlier times).
pub fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn from_unix_secs(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// Serde helpers storing a `SystemTime` as integer Unix seconds.
pub mod serde_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::SystemTime;

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(super::unix_secs(*t))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        Ok(super::from_unix_secs(u64::deserialize(d)?))
    }

    /// Same for `Option<SystemTime>`.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};
        use std::time::SystemTime;

        pub fn serialize<S: Serializer>(t: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
            match t {
                Some(t) => s.serialize_some(&crate::clock::unix_secs(*t)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<SystemTime>, D::Error> {
            Ok(Option::<u64>::deserialize(d)?.map(crate::clock::from_unix_secs))
        }
    }
}

/// Serde helpers storing a `Duration` as integer seconds.
pub mod serde_dur {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }

    /// Same for `Option<Duration>`.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};
        use std::time::Duration;

        pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
            match d {
                Some(d) => s.serialize_some(&d.as_secs()),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
            Ok(Option::<u64>::deserialize(d)?.map(Duration::from_secs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_sleep_advances_instantly() {
        let clock = ManualClock::fixed();
        let start = clock.now();
        clock
            .sleep(Duration::from_secs(20), &CancelToken::new())
            .unwrap();
        assert_eq!(
            clock.now().duration_since(start).unwrap(),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn cancelled_sleep_returns_cancelled() {
        let token = CancelToken::new();
        token.cancel();
        let err = ManualClock::fixed()
            .sleep(Duration::from_secs(1), &token)
            .unwrap_err();
        assert!(err.is(crate::ErrorKind::Cancelled));
        let err = SystemClock
            .sleep(Duration::from_secs(5), &token)
            .unwrap_err();
        assert!(err.is(crate::ErrorKind::Cancelled));
    }

    #[test]
    fn a_cancel_and_the_point_of_no_return_exclude_each_other() {
        let token = CancelToken::new();
        assert!(token.cancel(), "a cancel before it applies");
        assert!(token.cancel(), "and keeps applying");
        assert!(token.commit().unwrap_err().is(crate::ErrorKind::Cancelled));
        assert!(token.is_cancelled());

        let token = CancelToken::new();
        let worker = token.clone();
        worker.commit().unwrap();
        assert!(!token.cancel(), "the caller learns it came too late");
        assert!(!worker.is_cancelled(), "and the work carries on");
        worker.check().unwrap();
        worker.commit().unwrap();
    }
}
