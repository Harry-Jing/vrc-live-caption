//! Retry policy and connection-epoch bookkeeping for one runtime generation.

use crate::error::{AppError, RetryDisposition};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE_RECONNECT_DELAY_MILLIS: u64 = 500;
const MAX_RECONNECT_BACKOFF_EXPONENT: u32 = 6;
const MAX_RECONNECT_DELAY_MILLIS: u64 = 30_000;
const MIN_RECONNECT_JITTER_PERCENT: u32 = 80;
const MAX_RECONNECT_JITTER_PERCENT: u32 = 120;
const BACKOFF_RESET_AFTER_STABLE_CONNECTION: Duration = Duration::from_secs(30);

pub(crate) fn reconnect_jitter_percent() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .subsec_nanos();
    let inclusive_range = MAX_RECONNECT_JITTER_PERCENT - MIN_RECONNECT_JITTER_PERCENT + 1;
    MIN_RECONNECT_JITTER_PERCENT + nanos % inclusive_range
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReconnectDecision {
    Retry { attempt: u32, delay: Duration },
    Terminal,
}

#[derive(Default)]
pub(crate) struct ReconnectSupervisor {
    connection_epoch: u64,
    consecutive_failures: u32,
    has_reached_running: bool,
}

impl ReconnectSupervisor {
    pub(crate) fn begin_connection_attempt(&mut self) -> u64 {
        self.connection_epoch = self.connection_epoch.saturating_add(1);
        self.connection_epoch
    }

    pub(crate) fn is_recovery(&self) -> bool {
        self.has_reached_running
    }

    pub(crate) fn mark_running(&mut self) {
        self.has_reached_running = true;
    }

    pub(crate) fn on_failure(
        &mut self,
        error: &AppError,
        connected_for: Option<Duration>,
        jitter_percent: u32,
    ) -> ReconnectDecision {
        if error.retry_disposition() == RetryDisposition::Terminal {
            return ReconnectDecision::Terminal;
        }

        if connected_for.is_some_and(|duration| duration >= BACKOFF_RESET_AFTER_STABLE_CONNECTION) {
            self.consecutive_failures = 0;
        }

        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let exponent = self
            .consecutive_failures
            .saturating_sub(1)
            .min(MAX_RECONNECT_BACKOFF_EXPONENT);
        let multiplier = 1_u64 << exponent;
        let base_millis = BASE_RECONNECT_DELAY_MILLIS
            .saturating_mul(multiplier)
            .min(MAX_RECONNECT_DELAY_MILLIS);
        let jitter_percent = u64::from(
            jitter_percent.clamp(MIN_RECONNECT_JITTER_PERCENT, MAX_RECONNECT_JITTER_PERCENT),
        );
        let delay_millis = base_millis
            .saturating_mul(jitter_percent)
            .saturating_div(100)
            .min(MAX_RECONNECT_DELAY_MILLIS);

        ReconnectDecision::Retry {
            attempt: self.consecutive_failures,
            delay: Duration::from_millis(delay_millis),
        }
    }
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
