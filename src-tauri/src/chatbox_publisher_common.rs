//! Shared leaf-level mechanics for the independent Chatbox publishers.
//!
//! Policy-specific queue and candidate selection deliberately remain in their
//! concrete publishers. This module contains only vocabulary and worker
//! mechanics whose behavior is identical for Completed and Live publication.

use crate::chatbox_layout::ChatboxLayoutError;
use crate::error::{AppError, AppResult};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

// VRChat auto-hides its OSC typing indicator after about five seconds without
// fresh input. Reassert `true` every four seconds while activity remains active
// so scheduler jitter does not create a visible gap. Typing packets deliberately
// bypass ChatboxPacer and never consume a `/chatbox/input` text-send opportunity.
pub(crate) const TYPING_REASSERT_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublisherCloseReason {
    Stop,
    RuntimeError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum PublisherSubmitOutcome {
    Handled,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublisherLifecycle {
    Running,
    Closing {
        reason: PublisherCloseReason,
        cleanup_attempted: bool,
    },
    Closed,
}

struct PublisherJoinState {
    worker: Option<JoinHandle<AppResult<()>>>,
    failure: Option<String>,
}

#[derive(Clone)]
pub(crate) struct PublisherWorkerJoin {
    publisher_name: &'static str,
    state: Arc<Mutex<PublisherJoinState>>,
}

impl PublisherWorkerJoin {
    pub(crate) fn new(publisher_name: &'static str, worker: JoinHandle<AppResult<()>>) -> Self {
        Self {
            publisher_name,
            state: Arc::new(Mutex::new(PublisherJoinState {
                worker: Some(worker),
                failure: None,
            })),
        }
    }

    pub(crate) fn join(&self) -> AppResult<()> {
        let mut state = self.state.lock().map_err(|_| {
            AppError::state(format!(
                "{} publisher join lock was poisoned.",
                self.publisher_name
            ))
        })?;

        if let Some(worker) = state.worker.take() {
            let result = worker.join().map_err(|_| {
                AppError::runtime(format!(
                    "{} publisher worker panicked while stopping.",
                    self.publisher_name
                ))
            });
            let result = match result {
                Ok(worker_result) => worker_result,
                Err(error) => Err(error),
            };

            if let Err(error) = result {
                state.failure = Some(error.to_string());
            }
        }

        match &state.failure {
            Some(failure) => Err(AppError::runtime(format!(
                "{} publisher worker failed: {failure}",
                self.publisher_name
            ))),
            None => Ok(()),
        }
    }
}

pub(crate) fn describe_layout_error(error: ChatboxLayoutError) -> String {
    match error {
        ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units } => format!(
            "One grapheme requires {utf16_units} UTF-16 units, exceeding the 144-unit Chatbox input budget."
        ),
    }
}
