use super::text_pacing::Clock;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) struct AdvancingClock {
    now: Mutex<Instant>,
    sleeps: Mutex<Vec<Duration>>,
}

impl AdvancingClock {
    pub(super) fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn total_sleep(&self) -> Duration {
        self.sleeps
            .lock()
            .map(|sleeps| sleeps.iter().copied().sum())
            .unwrap_or_default()
    }
}

impl Clock for AdvancingClock {
    fn now(&self) -> Instant {
        self.now
            .lock()
            .map(|now| *now)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }

    fn sleep(&self, duration: Duration) {
        if let Ok(mut sleeps) = self.sleeps.lock() {
            sleeps.push(duration);
        }
        if let Ok(mut now) = self.now.lock() {
            *now += duration;
        }
    }
}
