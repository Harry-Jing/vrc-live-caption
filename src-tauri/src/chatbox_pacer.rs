//! Process-wide pacing for VRChat Chatbox text-send attempts.
//!
//! Typing-indicator packets are intentionally outside this module. The pacer
//! owns only the timing state shared by every `/chatbox/input` sender.

use crate::error::AppResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const PACING_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CHATBOX_ATTEMPT_INTERVAL: Duration = Duration::from_millis(1000);

pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

#[derive(Clone)]
pub(crate) struct ChatboxPacer {
    shared: Arc<SharedPacer>,
}

struct SharedPacer {
    clock: Arc<dyn Clock>,
    state: Mutex<PacerState>,
}

#[derive(Default)]
struct PacerState {
    last_attempt: Option<TextAttempt>,
}

#[derive(Clone, Copy)]
struct TextAttempt {
    started_at: Instant,
}

pub(crate) struct ChatboxAttemptPermit<'a> {
    clock: &'a dyn Clock,
    state: MutexGuard<'a, PacerState>,
}

impl Default for ChatboxPacer {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock))
    }
}

impl ChatboxPacer {
    fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            shared: Arc::new(SharedPacer {
                clock,
                state: Mutex::new(PacerState::default()),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self::new(clock)
    }

    pub(crate) fn now(&self) -> Instant {
        self.shared.clock.now()
    }

    /// Waits until one caller can decide whether to make the next text-send
    /// attempt. The returned permit keeps that decision exclusive; dropping
    /// it records nothing, while [`ChatboxAttemptPermit::attempt`] records the
    /// opportunity immediately before invoking transport.
    pub(crate) fn wait_for_turn(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<Option<ChatboxAttemptPermit<'_>>> {
        loop {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                return Ok(None);
            }

            let state =
                self.shared.state.lock().map_err(|_| {
                    crate::error::AppError::state("Chatbox pacer lock was poisoned.")
                })?;
            let now = self.shared.clock.now();
            let remaining = state.last_attempt.map(|last_attempt| {
                CHATBOX_ATTEMPT_INTERVAL
                    .saturating_sub(now.saturating_duration_since(last_attempt.started_at))
            });

            if let Some(remaining) = remaining.filter(|remaining| !remaining.is_zero()) {
                drop(state);
                self.shared.clock.sleep(remaining.min(PACING_POLL_INTERVAL));
                continue;
            }

            return Ok(Some(ChatboxAttemptPermit {
                clock: self.shared.clock.as_ref(),
                state,
            }));
        }
    }
}

impl ChatboxAttemptPermit<'_> {
    pub(crate) fn attempt<T>(mut self, attempt: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        self.state.last_attempt = Some(TextAttempt {
            started_at: self.clock.now(),
        });
        drop(self.state);

        attempt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    struct FakeClock {
        now: Mutex<Instant>,
        sleeps: Mutex<Vec<Duration>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                now: Mutex::new(Instant::now()),
                sleeps: Mutex::new(Vec::new()),
            }
        }

        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps
                .lock()
                .map(|sleeps| sleeps.clone())
                .unwrap_or_default()
        }
    }

    impl Clock for FakeClock {
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

    struct CancelOnSleepClock {
        clock: FakeClock,
        cancel: Arc<AtomicBool>,
    }

    impl Clock for CancelOnSleepClock {
        fn now(&self) -> Instant {
            self.clock.now()
        }

        fn sleep(&self, duration: Duration) {
            self.clock.sleep(duration);
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn first_actual_attempt_is_immediately_available() -> AppResult<()> {
        let clock = Arc::new(FakeClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        let cancel = AtomicBool::new(false);

        let permit = pacer
            .wait_for_turn(Some(&cancel))?
            .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?;
        permit.attempt(|| Ok(()))?;

        assert!(clock.sleeps().is_empty());

        Ok(())
    }

    #[test]
    fn actual_attempts_are_separated_by_the_fixed_one_second_interval() -> AppResult<()> {
        let clock = Arc::new(FakeClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        let cancel = AtomicBool::new(false);

        pacer
            .wait_for_turn(Some(&cancel))?
            .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?
            .attempt(|| Ok(()))?;
        pacer
            .wait_for_turn(Some(&cancel))?
            .ok_or_else(|| crate::error::AppError::runtime("Second attempt was cancelled."))?
            .attempt(|| Ok(()))?;

        assert_eq!(
            clock.sleeps().into_iter().sum::<Duration>(),
            Duration::from_secs(1)
        );

        Ok(())
    }

    #[test]
    fn failed_attempt_reserves_the_next_opportunity() -> AppResult<()> {
        let clock = Arc::new(FakeClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        let cancel = AtomicBool::new(false);

        let first_result = pacer
            .wait_for_turn(Some(&cancel))?
            .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?
            .attempt::<()>(|| {
                Err(crate::error::AppError::osc_send(
                    "test",
                    "failure".to_string(),
                ))
            });
        assert!(first_result.is_err());

        pacer
            .wait_for_turn(Some(&cancel))?
            .ok_or_else(|| crate::error::AppError::runtime("Second attempt was cancelled."))?
            .attempt(|| Ok(()))?;

        assert_eq!(
            clock.sleeps().into_iter().sum::<Duration>(),
            Duration::from_secs(1)
        );

        Ok(())
    }

    #[test]
    fn unused_permit_does_not_consume_an_attempt() -> AppResult<()> {
        let clock = Arc::new(FakeClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());

        let unused = pacer
            .wait_for_turn(None)?
            .ok_or_else(|| crate::error::AppError::runtime("Permit was cancelled."))?;
        drop(unused);
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| crate::error::AppError::runtime("Attempt was cancelled."))?
            .attempt(|| Ok(()))?;

        assert!(clock.sleeps().is_empty());

        Ok(())
    }

    #[test]
    fn concurrent_callers_never_consume_an_initial_burst() -> AppResult<()> {
        for _ in 0..100 {
            let clock = Arc::new(FakeClock::new());
            let pacer = ChatboxPacer::with_clock(clock.clone());
            let barrier = Arc::new(Barrier::new(3));
            let attempts = Arc::new(AtomicUsize::new(0));
            let mut workers = Vec::new();

            for _ in 0..2 {
                let worker_pacer = pacer.clone();
                let worker_barrier = barrier.clone();
                let worker_attempts = attempts.clone();
                workers.push(std::thread::spawn(move || -> AppResult<()> {
                    worker_barrier.wait();
                    worker_pacer
                        .wait_for_turn(None)?
                        .ok_or_else(|| crate::error::AppError::runtime("Attempt was cancelled."))?
                        .attempt(|| {
                            worker_attempts.fetch_add(1, Ordering::Relaxed);
                            Ok(())
                        })
                }));
            }

            barrier.wait();
            for worker in workers {
                worker.join().map_err(|_| {
                    crate::error::AppError::runtime("Pacer test worker panicked.")
                })??;
            }

            assert_eq!(attempts.load(Ordering::Relaxed), 2);
            assert_eq!(
                clock.sleeps().into_iter().sum::<Duration>(),
                Duration::from_secs(1)
            );
        }

        Ok(())
    }

    #[test]
    fn cancellation_during_wait_does_not_reserve_an_attempt() -> AppResult<()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let clock = Arc::new(CancelOnSleepClock {
            clock: FakeClock::new(),
            cancel: cancel.clone(),
        });
        let pacer = ChatboxPacer::with_clock(clock.clone());

        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?
            .attempt(|| Ok(()))?;
        assert!(pacer.wait_for_turn(Some(cancel.as_ref()))?.is_none());
        assert_eq!(
            clock.clock.sleeps().into_iter().sum::<Duration>(),
            PACING_POLL_INTERVAL
        );

        cancel.store(false, Ordering::Relaxed);
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| crate::error::AppError::runtime("Follow-up attempt was cancelled."))?
            .attempt(|| Ok(()))?;
        assert_eq!(
            clock.clock.sleeps().into_iter().sum::<Duration>(),
            CHATBOX_ATTEMPT_INTERVAL
        );

        Ok(())
    }
}
