//! Independent Completed publication for the VRChat Chatbox.
//!
//! Runtime producers submit Source-unit recognition lifecycle changes through
//! a non-waiting in-memory seam. One dedicated worker owns pagination output,
//! typing transitions, queue order, process-wide pacing, OSC attempts, and
//! diagnostics. No producer waits for a Chatbox text-send pacing opportunity
//! or network operation.

use super::PreparedChatboxText;
use super::common::{
    PublicationObservationOutcome, PublisherCloseReason, PublisherLifecycle, PublisherWorkerJoin,
    TYPING_REASSERT_INTERVAL, describe_layout_error,
};
use super::layout::prepare_completed_pages;
use super::text_pacing::{ChatboxTextAttemptPermit, ChatboxTextPacer};
use super::transport::ChatboxTransport;
use crate::caption::{CaptionAggregateChange, CaptionAggregateUpdate, CaptionLane, CaptionState};
use crate::error::{AppError, AppResult};
use crate::generation_fence::GenerationCommitter;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PROVISIONAL_MAX_RESIDENT_PAGES: usize = 32;
const PROVISIONAL_MAX_WAIT_BEFORE_FIRST_SEND_ATTEMPT: Duration = Duration::from_secs(30);

pub(crate) type CompletedPublisherReporter =
    Arc<dyn Fn(CompletedPublisherDiagnostic) + Send + Sync>;

#[derive(Debug)]
pub(crate) enum CompletedPublisherDiagnostic {
    UnitSendSucceeded {
        unit_id: String,
        page_count: usize,
        byte_count: usize,
        target: String,
    },
    UnitDroppedOverload {
        unit_id: String,
        page_count: usize,
    },
    UnitRejectedOverload {
        unit_id: String,
        page_count: usize,
    },
    UnitExpired {
        unit_id: String,
        page_count: usize,
    },
    LayoutFailed {
        unit_id: String,
        reason: String,
    },
    UnitSendFailed {
        unit_id: String,
        page_index: usize,
        page_count: usize,
        pages_sent: usize,
        error: AppError,
    },
    PagesDiscardedOnClose {
        reason: PublisherCloseReason,
        unit_count: usize,
        page_count: usize,
        send_started_unit_count: usize,
    },
    TypingFailed {
        is_typing: bool,
        error: AppError,
    },
    WorkerFailed {
        reason: String,
    },
}

enum SourceUnitEvent {
    Opened { unit_id: String },
    Completed { unit_id: String, text: String },
    Aborted { unit_id: String },
}

#[derive(Clone)]
pub(crate) struct CompletedChatboxPublisher {
    shared: Arc<PublisherShared>,
    worker_join: PublisherWorkerJoin,
}

#[derive(Clone, Copy)]
struct PublisherLimits {
    max_resident_pages: usize,
    max_wait_before_first_send_attempt: Duration,
}

struct PublisherShared {
    state: Mutex<PublisherState>,
    wake: Condvar,
    interrupt_text_wait: AtomicBool,
    transport: Arc<dyn ChatboxTransport>,
    text_pacer: ChatboxTextPacer,
    committer: GenerationCommitter,
    reporter: CompletedPublisherReporter,
    limits: PublisherLimits,
}

struct PublisherState {
    lifecycle: PublisherLifecycle,
    units: VecDeque<QueuedUnitPublication>,
    resident_pages: usize,
    next_sequence: u64,
    // Unit IDs that keep the typing indicator active until the unit aborts or
    // its Completed publication resolves.
    typing_active_units: HashSet<String>,
    typing_desired: bool,
    typing_epoch: u64,
    typing_attempted_epoch: Option<u64>,
    next_typing_reassert_at: Option<Instant>,
    diagnostics: VecDeque<CompletedPublisherDiagnostic>,
}

struct QueuedUnitPublication {
    sequence: u64,
    unit_id: String,
    pages: Vec<PreparedChatboxText>,
    next_page: usize,
    first_send_attempt_started: bool,
    enqueued_at: Instant,
    sent_pages: usize,
    sent_bytes: usize,
    target: Option<String>,
}

impl QueuedUnitPublication {
    fn remaining_pages(&self) -> usize {
        self.pages.len().saturating_sub(self.next_page)
    }
}

enum WorkerItem {
    Typing {
        epoch: u64,
        is_typing: bool,
    },
    CleanupTyping,
    Diagnostic(CompletedPublisherDiagnostic),
    Page {
        sequence: u64,
        page_index: usize,
        text: PreparedChatboxText,
    },
    Exit,
}

impl CompletedChatboxPublisher {
    pub(crate) fn start(
        transport: Arc<dyn ChatboxTransport>,
        text_pacer: ChatboxTextPacer,
        committer: GenerationCommitter,
        reporter: CompletedPublisherReporter,
    ) -> AppResult<Self> {
        Self::start_with_limits(
            transport,
            text_pacer,
            committer,
            reporter,
            PublisherLimits {
                max_resident_pages: PROVISIONAL_MAX_RESIDENT_PAGES,
                max_wait_before_first_send_attempt: PROVISIONAL_MAX_WAIT_BEFORE_FIRST_SEND_ATTEMPT,
            },
        )
    }

    fn start_with_limits(
        transport: Arc<dyn ChatboxTransport>,
        text_pacer: ChatboxTextPacer,
        committer: GenerationCommitter,
        reporter: CompletedPublisherReporter,
        limits: PublisherLimits,
    ) -> AppResult<Self> {
        if limits.max_resident_pages == 0 || limits.max_wait_before_first_send_attempt.is_zero() {
            return Err(AppError::state(
                "Completed publisher limits must both be greater than zero.",
            ));
        }

        let shared = Arc::new(PublisherShared {
            state: Mutex::new(PublisherState {
                lifecycle: PublisherLifecycle::Running,
                units: VecDeque::new(),
                resident_pages: 0,
                next_sequence: 1,
                typing_active_units: HashSet::new(),
                typing_desired: false,
                typing_epoch: 0,
                // Epoch zero represents the initial typing-off state; it does
                // not need a transport transition.
                typing_attempted_epoch: Some(0),
                next_typing_reassert_at: None,
                diagnostics: VecDeque::new(),
            }),
            wake: Condvar::new(),
            interrupt_text_wait: AtomicBool::new(false),
            transport,
            text_pacer,
            committer,
            reporter,
            limits,
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("vrc-live-caption-completed-publisher".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_publisher_worker(Arc::clone(&worker_shared))
                }));
                match result {
                    Ok(worker_result) => {
                        if let Err(error) = &worker_result {
                            emergency_close_after_worker_failure(
                                &worker_shared,
                                format!("Completed publisher worker failed: {error}"),
                            );
                        }
                        worker_result
                    }
                    Err(panic) => {
                        emergency_close_after_worker_failure(
                            &worker_shared,
                            "Completed publisher worker panicked.".to_string(),
                        );
                        std::panic::resume_unwind(panic);
                    }
                }
            })
            .map_err(|error| {
                AppError::runtime(format!(
                    "Failed to start Completed publisher worker: {error}"
                ))
            })?;

        Ok(Self {
            shared,
            worker_join: PublisherWorkerJoin::new("Completed", worker),
        })
    }

    /// Translates one accepted aggregate change into the existing lifecycle.
    pub(crate) fn try_observe(
        &self,
        update: &CaptionAggregateUpdate,
    ) -> AppResult<PublicationObservationOutcome> {
        let input = match &update.change {
            CaptionAggregateChange::SourceUnitOpened(unit) => Some(SourceUnitEvent::Opened {
                unit_id: unit.unit_id.clone(),
            }),
            CaptionAggregateChange::SourceUnitAborted { unit_id } => {
                Some(SourceUnitEvent::Aborted {
                    unit_id: unit_id.clone(),
                })
            }
            CaptionAggregateChange::CaptionAccepted(caption)
                if caption.lane == CaptionLane::Source
                    && caption.state == CaptionState::Completed
                    && update
                        .snapshot
                        .active_stream
                        .as_ref()
                        .is_some_and(|active| {
                            caption.generation == active.generation
                                && caption.stream_id == active.stream_id
                        }) =>
            {
                caption
                    .unit_id
                    .as_ref()
                    .map(|unit_id| SourceUnitEvent::Completed {
                        unit_id: unit_id.clone(),
                        text: caption.text.clone(),
                    })
            }
            CaptionAggregateChange::CaptionAccepted(_) => None,
            CaptionAggregateChange::TranslationFailed(_) => None,
        };

        match input {
            Some(input) => self.try_handle_input(input),
            None => {
                let state = self.lock_state()?;
                Ok(
                    if state.lifecycle == PublisherLifecycle::Running
                        && !self.shared.committer.is_closed()
                    {
                        PublicationObservationOutcome::Handled
                    } else {
                        PublicationObservationOutcome::Closed
                    },
                )
            }
        }
    }

    /// Applies one complete lifecycle event without waiting for pacing or OSC.
    fn try_handle_input(&self, event: SourceUnitEvent) -> AppResult<PublicationObservationOutcome> {
        match event {
            SourceUnitEvent::Opened { unit_id } => {
                let mut state = self.lock_state()?;
                if state.lifecycle != PublisherLifecycle::Running
                    || self.shared.committer.is_closed()
                {
                    return Ok(PublicationObservationOutcome::Closed);
                }

                if state.typing_active_units.insert(unit_id) {
                    refresh_typing_desired(&mut state);
                    self.signal_worker_locked();
                }
            }
            SourceUnitEvent::Aborted { unit_id } => {
                let mut state = self.lock_state()?;
                if state.lifecycle != PublisherLifecycle::Running
                    || self.shared.committer.is_closed()
                {
                    return Ok(PublicationObservationOutcome::Closed);
                }

                release_unit_typing_activity(&mut state, &unit_id);
                self.signal_worker_locked();
            }
            SourceUnitEvent::Completed { unit_id, text } => {
                return self.try_enqueue_completed_source(unit_id, text);
            }
        }

        Ok(PublicationObservationOutcome::Handled)
    }

    /// Closes admission and wakes the worker to discard every resident page.
    /// The worker performs the one allowed typing-off cleanup before it exits.
    pub(crate) fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        self.shared
            .interrupt_text_wait
            .store(true, Ordering::SeqCst);
        let (mut state, state_was_poisoned) = match self.shared.state.lock() {
            Ok(state) => (state, false),
            Err(poisoned) => (poisoned.into_inner(), true),
        };

        match state.lifecycle {
            PublisherLifecycle::Running => {
                state.lifecycle = PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted: false,
                };
            }
            PublisherLifecycle::Closing {
                reason: current_reason,
                cleanup_attempted,
            } => {
                if reason == PublisherCloseReason::Stop
                    && current_reason == PublisherCloseReason::RuntimeError
                {
                    state.lifecycle = PublisherLifecycle::Closing {
                        reason,
                        cleanup_attempted,
                    };
                }
            }
            PublisherLifecycle::Closed => {}
        }

        // Repeat this while holding the state lock. Otherwise a worker that
        // selected a page between the first store and this lock acquisition
        // could overwrite the interrupt and postpone Stop cleanup.
        self.shared
            .interrupt_text_wait
            .store(true, Ordering::SeqCst);
        let perform_poison_cleanup = if state_was_poisoned {
            match state.lifecycle {
                PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted: false,
                } => {
                    discard_resident_pages_on_close(&mut state, reason);
                    state.lifecycle = PublisherLifecycle::Closing {
                        reason,
                        cleanup_attempted: true,
                    };
                    true
                }
                PublisherLifecycle::Running
                | PublisherLifecycle::Closing {
                    cleanup_attempted: true,
                    ..
                }
                | PublisherLifecycle::Closed => false,
            }
        } else {
            false
        };
        self.shared.wake.notify_all();
        drop(state);

        if state_was_poisoned {
            let cleanup_note = if perform_poison_cleanup {
                match self.shared.transport.send_typing(false) {
                    Ok(()) => " A best-effort typing-off cleanup was attempted.",
                    Err(_) => " The best-effort typing-off cleanup also failed.",
                }
            } else {
                ""
            };
            Err(AppError::state(format!(
                "Completed publisher state lock was poisoned while closing; shutdown was still requested.{cleanup_note}"
            )))
        } else {
            Ok(())
        }
    }

    pub(crate) fn join(&self) -> AppResult<()> {
        self.worker_join.join()
    }

    #[cfg(test)]
    pub(crate) fn wait_until_text_quiescent_for_test(&self, timeout: Duration) -> AppResult<()> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut state = self.lock_state()?;
        loop {
            if state.lifecycle != PublisherLifecycle::Running {
                return Err(AppError::state(
                    "Completed publisher closed before text became quiescent.",
                ));
            }
            // A selected or in-flight page remains resident until its transport
            // attempt has returned, so an empty queue is a causal text barrier.
            if state.units.is_empty() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::state(
                    "Completed publisher text did not quiesce before the test watchdog expired.",
                ));
            }
            let (next_state, wait_result) = self
                .shared
                .wake
                .wait_timeout(state, remaining)
                .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
            state = next_state;
            if wait_result.timed_out() && !state.units.is_empty() {
                return Err(AppError::state(
                    "Completed publisher text did not quiesce before the test watchdog expired.",
                ));
            }
        }
    }

    fn try_enqueue_completed_source(
        &self,
        unit_id: String,
        text: String,
    ) -> AppResult<PublicationObservationOutcome> {
        let pages = match prepare_completed_pages(&text) {
            Ok(pages) => pages,
            Err(error) => {
                let mut state = self.lock_state()?;
                if state.lifecycle != PublisherLifecycle::Running
                    || self.shared.committer.is_closed()
                {
                    return Ok(PublicationObservationOutcome::Closed);
                }
                release_unit_typing_activity(&mut state, &unit_id);
                state
                    .diagnostics
                    .push_back(CompletedPublisherDiagnostic::LayoutFailed {
                        unit_id,
                        reason: describe_layout_error(error),
                    });
                self.signal_worker_locked();
                return Ok(PublicationObservationOutcome::Handled);
            }
        };

        let mut state = self.lock_state()?;
        if state.lifecycle != PublisherLifecycle::Running || self.shared.committer.is_closed() {
            return Ok(PublicationObservationOutcome::Closed);
        }

        if pages.is_empty() {
            release_unit_typing_activity(&mut state, &unit_id);
            self.signal_worker_locked();
            return Ok(PublicationObservationOutcome::Handled);
        }

        let now = self.shared.text_pacer.now();
        expire_units_waiting_for_first_send_attempt(
            &mut state,
            now,
            self.shared.limits.max_wait_before_first_send_attempt,
        )?;
        let page_count = pages.len();
        let protected_pages = state
            .units
            .front()
            .filter(|unit| unit.first_send_attempt_started)
            .map(QueuedUnitPublication::remaining_pages)
            .unwrap_or(0);

        if page_count > self.shared.limits.max_resident_pages
            || protected_pages.saturating_add(page_count) > self.shared.limits.max_resident_pages
        {
            release_unit_typing_activity(&mut state, &unit_id);
            state
                .diagnostics
                .push_back(CompletedPublisherDiagnostic::UnitRejectedOverload {
                    unit_id,
                    page_count,
                });
            self.signal_worker_locked();
            return Ok(PublicationObservationOutcome::Handled);
        }

        while state.resident_pages.saturating_add(page_count)
            > self.shared.limits.max_resident_pages
        {
            let Some(position) = state
                .units
                .iter()
                .position(|unit| !unit.first_send_attempt_started)
            else {
                release_unit_typing_activity(&mut state, &unit_id);
                state
                    .diagnostics
                    .push_back(CompletedPublisherDiagnostic::UnitRejectedOverload {
                        unit_id,
                        page_count,
                    });
                self.signal_worker_locked();
                return Ok(PublicationObservationOutcome::Handled);
            };
            let Some(dropped) = state.units.remove(position) else {
                return Err(AppError::state(
                    "Completed publisher could not remove an overload candidate.",
                ));
            };
            let dropped_pages = dropped.remaining_pages();
            state.resident_pages = state
                .resident_pages
                .checked_sub(dropped_pages)
                .ok_or_else(|| AppError::state("Completed publisher page count underflowed."))?;
            release_unit_typing_activity(&mut state, &dropped.unit_id);
            state
                .diagnostics
                .push_back(CompletedPublisherDiagnostic::UnitDroppedOverload {
                    unit_id: dropped.unit_id,
                    page_count: dropped_pages,
                });
        }

        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        state.resident_pages += page_count;
        state.units.push_back(QueuedUnitPublication {
            sequence,
            unit_id,
            pages,
            next_page: 0,
            first_send_attempt_started: false,
            enqueued_at: now,
            sent_pages: 0,
            sent_bytes: 0,
            target: None,
        });
        self.signal_worker_locked();

        Ok(PublicationObservationOutcome::Handled)
    }

    fn lock_state(&self) -> AppResult<std::sync::MutexGuard<'_, PublisherState>> {
        self.shared
            .state
            .lock()
            .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))
    }

    fn signal_worker_locked(&self) {
        self.shared
            .interrupt_text_wait
            .store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
    }
}

fn run_publisher_worker(shared: Arc<PublisherShared>) -> AppResult<()> {
    loop {
        match next_worker_item(&shared)? {
            WorkerItem::Typing { epoch, is_typing } => {
                process_typing(&shared, epoch, is_typing)?;
            }
            WorkerItem::CleanupTyping => process_cleanup_typing(&shared)?,
            WorkerItem::Diagnostic(diagnostic) => (shared.reporter)(diagnostic),
            WorkerItem::Page {
                sequence,
                page_index,
                text,
            } => {
                let permit = shared
                    .text_pacer
                    .wait_for_text_attempt(Some(&shared.interrupt_text_wait))?;
                let Some(permit) = permit else {
                    continue;
                };
                let attempt_result = shared.committer.try_commit(|| {
                    attempt_selected_page(&shared, sequence, page_index, &text, permit)
                })?;

                if let Some(result) = attempt_result {
                    result?;
                } else {
                    thread::yield_now();
                }
            }
            WorkerItem::Exit => return Ok(()),
        }
    }
}

fn emergency_close_after_worker_failure(shared: &PublisherShared, failure_reason: String) {
    shared.interrupt_text_wait.store(true, Ordering::SeqCst);
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    if state.lifecycle == PublisherLifecycle::Closed {
        return;
    }
    let cleanup_already_attempted = matches!(
        state.lifecycle,
        PublisherLifecycle::Closing {
            cleanup_attempted: true,
            ..
        }
    );
    let reason = match state.lifecycle {
        PublisherLifecycle::Closing { reason, .. } => reason,
        PublisherLifecycle::Running | PublisherLifecycle::Closed => {
            PublisherCloseReason::RuntimeError
        }
    };
    discard_resident_pages_on_close(&mut state, reason);
    state.lifecycle = PublisherLifecycle::Closed;
    let mut diagnostics = state.diagnostics.drain(..).collect::<Vec<_>>();
    diagnostics.push(CompletedPublisherDiagnostic::WorkerFailed {
        reason: failure_reason,
    });
    drop(state);

    if !cleanup_already_attempted && let Err(error) = shared.transport.send_typing(false) {
        diagnostics.push(CompletedPublisherDiagnostic::TypingFailed {
            is_typing: false,
            error,
        });
    }
    for diagnostic in diagnostics {
        (shared.reporter)(diagnostic);
    }
    shared.wake.notify_all();
}

fn next_worker_item(shared: &PublisherShared) -> AppResult<WorkerItem> {
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;

    loop {
        match state.lifecycle {
            PublisherLifecycle::Closing {
                reason,
                cleanup_attempted: false,
            } => {
                discard_resident_pages_on_close(&mut state, reason);
                state.lifecycle = PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted: true,
                };
                return Ok(WorkerItem::CleanupTyping);
            }
            PublisherLifecycle::Closing {
                cleanup_attempted: true,
                ..
            } => {
                if let Some(diagnostic) = state.diagnostics.pop_front() {
                    return Ok(WorkerItem::Diagnostic(diagnostic));
                }
                state.lifecycle = PublisherLifecycle::Closed;
                shared.wake.notify_all();
                return Ok(WorkerItem::Exit);
            }
            PublisherLifecycle::Closed => return Ok(WorkerItem::Exit),
            PublisherLifecycle::Running => {}
        }

        expire_units_waiting_for_first_send_attempt(
            &mut state,
            shared.text_pacer.now(),
            shared.limits.max_wait_before_first_send_attempt,
        )?;

        if state.typing_attempted_epoch != Some(state.typing_epoch) {
            let epoch = state.typing_epoch;
            let is_typing = state.typing_desired;
            state.typing_attempted_epoch = Some(epoch);
            return Ok(WorkerItem::Typing { epoch, is_typing });
        }

        if state.typing_desired
            && state
                .next_typing_reassert_at
                .is_some_and(|deadline| shared.text_pacer.now() >= deadline)
        {
            return Ok(WorkerItem::Typing {
                epoch: state.typing_epoch,
                is_typing: true,
            });
        }

        if let Some(diagnostic) = state.diagnostics.pop_front() {
            return Ok(WorkerItem::Diagnostic(diagnostic));
        }

        if let Some(unit) = state.units.front() {
            let Some(text) = unit.pages.get(unit.next_page).cloned() else {
                return Err(AppError::state(
                    "Completed publisher unit had no current page.",
                ));
            };
            let item = WorkerItem::Page {
                sequence: unit.sequence,
                page_index: unit.next_page,
                text,
            };
            shared.interrupt_text_wait.store(false, Ordering::SeqCst);
            return Ok(item);
        }

        if let Some(deadline) = state.next_typing_reassert_at {
            let remaining = deadline.saturating_duration_since(shared.text_pacer.now());
            let (next_state, _) = shared
                .wake
                .wait_timeout(state, remaining)
                .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
            state = next_state;
        } else {
            state = shared
                .wake
                .wait(state)
                .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
        }
    }
}

fn process_typing(shared: &PublisherShared, epoch: u64, is_typing: bool) -> AppResult<()> {
    let transport_result = shared.committer.try_commit(|| {
        let state = shared
            .state
            .lock()
            .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
        let should_attempt = state.lifecycle == PublisherLifecycle::Running
            && state.typing_epoch == epoch
            && state.typing_desired == is_typing;
        drop(state);
        Ok(should_attempt.then(|| shared.transport.send_typing(is_typing)))
    })?;

    let Some(transport_result) = transport_result else {
        thread::yield_now();
        return Ok(());
    };

    let Some(result) = transport_result? else {
        return Ok(());
    };
    let attempted_at = shared.text_pacer.now();
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;

    if is_typing
        && state.lifecycle == PublisherLifecycle::Running
        && state.typing_epoch == epoch
        && state.typing_desired
    {
        state.next_typing_reassert_at = Some(attempted_at + TYPING_REASSERT_INTERVAL);
    }

    if let Err(error) = result {
        state
            .diagnostics
            .push_back(CompletedPublisherDiagnostic::TypingFailed { is_typing, error });
    }
    shared.wake.notify_all();

    Ok(())
}

fn process_cleanup_typing(shared: &PublisherShared) -> AppResult<()> {
    let result = shared.transport.send_typing(false);
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;

    if let Err(error) = result {
        state
            .diagnostics
            .push_back(CompletedPublisherDiagnostic::TypingFailed {
                is_typing: false,
                error,
            });
    }
    shared.wake.notify_all();

    Ok(())
}

fn discard_resident_pages_on_close(state: &mut PublisherState, reason: PublisherCloseReason) {
    let unit_count = state.units.len();
    let page_count = state.resident_pages;
    let send_started_unit_count = state
        .units
        .iter()
        .filter(|unit| unit.first_send_attempt_started)
        .count();

    state.units.clear();
    state.resident_pages = 0;
    state.typing_active_units.clear();
    state.typing_desired = false;
    state.typing_epoch = state.typing_epoch.wrapping_add(1);
    state.typing_attempted_epoch = None;
    state.next_typing_reassert_at = None;

    if page_count > 0 {
        state
            .diagnostics
            .push_back(CompletedPublisherDiagnostic::PagesDiscardedOnClose {
                reason,
                unit_count,
                page_count,
                send_started_unit_count,
            });
    }
}

fn attempt_selected_page(
    shared: &PublisherShared,
    sequence: u64,
    page_index: usize,
    text: &PreparedChatboxText,
    permit: ChatboxTextAttemptPermit<'_>,
) -> AppResult<()> {
    {
        let mut state = shared
            .state
            .lock()
            .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
        if state.lifecycle != PublisherLifecycle::Running {
            return Ok(());
        }

        let Some(unit) = state.units.front_mut() else {
            return Ok(());
        };
        if unit.sequence != sequence || unit.next_page != page_index {
            return Ok(());
        }

        if !unit.first_send_attempt_started
            && shared
                .text_pacer
                .now()
                .saturating_duration_since(unit.enqueued_at)
                >= shared.limits.max_wait_before_first_send_attempt
        {
            let Some(expired) = state.units.pop_front() else {
                return Ok(());
            };
            let expired_pages = expired.remaining_pages();
            state.resident_pages = state
                .resident_pages
                .checked_sub(expired_pages)
                .ok_or_else(|| AppError::state("Completed publisher page count underflowed."))?;
            release_unit_typing_activity(&mut state, &expired.unit_id);
            state
                .diagnostics
                .push_back(CompletedPublisherDiagnostic::UnitExpired {
                    unit_id: expired.unit_id,
                    page_count: expired_pages,
                });
            shared.wake.notify_all();
            return Ok(());
        }

        unit.first_send_attempt_started = true;
    }

    let send_result = permit.attempt(|| shared.transport.send_text(text));
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
    let is_current = state
        .units
        .front()
        .is_some_and(|unit| unit.sequence == sequence && unit.next_page == page_index);
    if !is_current {
        return Ok(());
    }

    match send_result {
        Ok(receipt) => {
            state.resident_pages = state
                .resident_pages
                .checked_sub(1)
                .ok_or_else(|| AppError::state("Completed publisher page count underflowed."))?;
            let Some(unit) = state.units.front_mut() else {
                return Ok(());
            };
            unit.next_page += 1;
            unit.sent_pages += 1;
            unit.sent_bytes = unit.sent_bytes.saturating_add(receipt.byte_count);
            unit.target = Some(receipt.target);

            if unit.next_page == unit.pages.len() {
                let Some(completed) = state.units.pop_front() else {
                    return Ok(());
                };
                release_unit_typing_activity(&mut state, &completed.unit_id);
                state
                    .diagnostics
                    .push_back(CompletedPublisherDiagnostic::UnitSendSucceeded {
                        unit_id: completed.unit_id,
                        page_count: completed.pages.len(),
                        byte_count: completed.sent_bytes,
                        target: completed.target.unwrap_or_else(|| "unknown".to_string()),
                    });
            }
        }
        Err(error) => {
            let Some(failed) = state.units.pop_front() else {
                return Ok(());
            };
            let remaining_pages = failed.remaining_pages();
            state.resident_pages = state
                .resident_pages
                .checked_sub(remaining_pages)
                .ok_or_else(|| AppError::state("Completed publisher page count underflowed."))?;
            release_unit_typing_activity(&mut state, &failed.unit_id);
            state
                .diagnostics
                .push_back(CompletedPublisherDiagnostic::UnitSendFailed {
                    unit_id: failed.unit_id,
                    page_index: page_index + 1,
                    page_count: failed.pages.len(),
                    pages_sent: failed.sent_pages,
                    error,
                });
        }
    }
    shared.wake.notify_all();

    Ok(())
}

fn expire_units_waiting_for_first_send_attempt(
    state: &mut PublisherState,
    now: Instant,
    max_wait_before_first_send_attempt: Duration,
) -> AppResult<()> {
    loop {
        let position = state.units.iter().position(|unit| {
            !unit.first_send_attempt_started
                && now.saturating_duration_since(unit.enqueued_at)
                    >= max_wait_before_first_send_attempt
        });
        let Some(position) = position else {
            return Ok(());
        };
        let Some(expired) = state.units.remove(position) else {
            return Err(AppError::state(
                "Completed publisher could not remove an expired unit.",
            ));
        };
        let expired_pages = expired.remaining_pages();
        state.resident_pages = state
            .resident_pages
            .checked_sub(expired_pages)
            .ok_or_else(|| AppError::state("Completed publisher page count underflowed."))?;
        release_unit_typing_activity(state, &expired.unit_id);
        state
            .diagnostics
            .push_back(CompletedPublisherDiagnostic::UnitExpired {
                unit_id: expired.unit_id,
                page_count: expired_pages,
            });
    }
}

fn release_unit_typing_activity(state: &mut PublisherState, unit_id: &str) {
    if state.typing_active_units.remove(unit_id) {
        refresh_typing_desired(state);
    }
}

fn refresh_typing_desired(state: &mut PublisherState) {
    let desired = !state.typing_active_units.is_empty();
    if desired != state.typing_desired {
        state.typing_desired = desired;
        state.typing_epoch = state.typing_epoch.wrapping_add(1);
        state.typing_attempted_epoch = None;
        state.next_typing_reassert_at = None;
    }
}

#[cfg(test)]
#[path = "completed_tests.rs"]
mod tests;
