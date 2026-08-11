//! Generation-scoped output commit linearization.
//!
//! Runtime owns the fence and its close authority. Output sinks receive only a
//! [`GenerationCommitter`], which can observe closure and attempt work inside
//! the generation's commit boundary.

use crate::error::{AppError, AppResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct GenerationFenceState {
    gate: Mutex<()>,
    closed: AtomicBool,
    stop_requested: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct GenerationFence {
    state: Arc<GenerationFenceState>,
}

#[derive(Clone)]
pub(crate) struct GenerationCommitter {
    state: Arc<GenerationFenceState>,
}

impl GenerationFence {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(GenerationFenceState {
                gate: Mutex::new(()),
                closed: AtomicBool::new(false),
                stop_requested: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn committer(&self) -> GenerationCommitter {
        GenerationCommitter {
            state: Arc::clone(&self.state),
        }
    }

    pub(crate) fn close_admission(&self) {
        self.state.closed.store(true, Ordering::SeqCst);
    }

    /// Establishes the hard-Stop commit cutoff without waiting for an older
    /// commit. A commit that already passed the in-gate check may finish; every
    /// later App or sink commit observes the same generation-scoped boundary.
    pub(crate) fn request_stop(&self) {
        self.state.stop_requested.store(true, Ordering::SeqCst);
    }

    pub(crate) fn is_stop_requested(&self) -> bool {
        self.state.stop_requested.load(Ordering::SeqCst)
    }

    pub(crate) fn wait_for_commits(&self) -> AppResult<()> {
        match self.state.gate.lock() {
            Ok(_gate) => Ok(()),
            Err(poisoned) => {
                drop(poisoned.into_inner());
                Err(AppError::state(
                    "Runtime generation commit gate was poisoned.",
                ))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        if let Ok(_gate) = self.state.gate.lock() {
            std::panic::resume_unwind(Box::new("poison generation commit gate"));
        }
    }
}

impl GenerationCommitter {
    pub(crate) fn is_stop_requested(&self) -> bool {
        self.state.stop_requested.load(Ordering::SeqCst)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::SeqCst) || self.is_stop_requested()
    }

    pub(crate) fn try_commit<T>(&self, commit: impl FnOnce() -> T) -> AppResult<Option<T>> {
        let _gate = self
            .state
            .gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation commit gate was poisoned."))?;

        if self.is_closed() {
            return Ok(None);
        }

        Ok(Some(commit()))
    }
}
