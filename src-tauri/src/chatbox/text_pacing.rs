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
const CHATBOX_TEXT_ATTEMPT_INTERVAL: Duration = Duration::from_millis(1000);

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
pub(crate) struct ChatboxTextPacer {
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

pub(crate) struct ChatboxTextAttemptPermit<'a> {
    clock: &'a dyn Clock,
    state: MutexGuard<'a, PacerState>,
}

impl Default for ChatboxTextPacer {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock))
    }
}

impl ChatboxTextPacer {
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
    /// it records nothing, while [`ChatboxTextAttemptPermit::attempt`] records the
    /// opportunity immediately before invoking transport.
    pub(crate) fn wait_for_text_attempt(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<Option<ChatboxTextAttemptPermit<'_>>> {
        loop {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                return Ok(None);
            }

            let state = self.shared.state.lock().map_err(|_| {
                crate::error::AppError::state("Chatbox text-pacing lock was poisoned.")
            })?;
            let now = self.shared.clock.now();
            let remaining = state.last_attempt.map(|last_attempt| {
                CHATBOX_TEXT_ATTEMPT_INTERVAL
                    .saturating_sub(now.saturating_duration_since(last_attempt.started_at))
            });

            if let Some(remaining) = remaining.filter(|remaining| !remaining.is_zero()) {
                drop(state);
                self.shared.clock.sleep(remaining.min(PACING_POLL_INTERVAL));
                continue;
            }

            return Ok(Some(ChatboxTextAttemptPermit {
                clock: self.shared.clock.as_ref(),
                state,
            }));
        }
    }
}

impl ChatboxTextAttemptPermit<'_> {
    pub(crate) fn attempt<T>(mut self, attempt: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        self.state.last_attempt = Some(TextAttempt {
            started_at: self.clock.now(),
        });
        drop(self.state);

        attempt()
    }
}

#[cfg(test)]
#[path = "text_pacing_tests.rs"]
mod tests;
