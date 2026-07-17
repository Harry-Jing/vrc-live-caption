//! Independent Completed publication for the VRChat Chatbox.
//!
//! Runtime producers submit whole caption-unit lifecycle events through a
//! non-waiting in-memory seam. One dedicated worker owns pagination output,
//! typing transitions, queue order, process-wide pacing, OSC attempts, and
//! diagnostics. No producer waits for a Chatbox pacing opportunity or network
//! operation.

use crate::chatbox_layout::{ChatboxLayoutError, paginate_completed};
use crate::chatbox_pacer::{ChatboxAttemptPermit, ChatboxPacer};
use crate::error::{AppError, AppResult};
use crate::runtime::RuntimeGeneration;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PROVISIONAL_MAX_RESIDENT_PAGES: usize = 32;
const PROVISIONAL_MAX_UNSTARTED_AGE: Duration = Duration::from_secs(30);
// VRChat auto-hides its OSC typing indicator after about five seconds without
// fresh input. Reassert `true` every four seconds while activity remains active
// so scheduler jitter does not create a visible gap. Typing packets deliberately
// bypass ChatboxPacer and never consume a `/chatbox/input` text-send opportunity.
const TYPING_REASSERT_INTERVAL: Duration = Duration::from_secs(4);

pub(crate) trait ChatboxTransport: Send + Sync {
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt>;
    fn send_typing(&self, is_typing: bool) -> AppResult<()>;
}

#[derive(Debug)]
pub(crate) struct ChatboxSendReceipt {
    pub(crate) target: String,
    pub(crate) byte_count: usize,
}

pub(crate) type PublisherReporter = Arc<dyn Fn(PublisherDiagnostic) + Send + Sync>;

#[derive(Debug)]
pub(crate) enum PublisherDiagnostic {
    UnitPublished {
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
        started_unit_count: usize,
    },
    TypingFailed {
        is_typing: bool,
        error: AppError,
    },
    WorkerFailed {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublisherCloseReason {
    Stop,
    RuntimeError,
}

pub(crate) enum CompletedPublisherEvent {
    Started { unit_id: String },
    Completed { unit_id: String, text: String },
    Aborted { unit_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum PublisherSubmitOutcome {
    Handled,
    Closed,
}

#[derive(Clone)]
pub(crate) struct CompletedChatboxPublisher {
    shared: Arc<PublisherShared>,
    join_state: Arc<Mutex<PublisherJoinState>>,
}

#[derive(Clone, Copy)]
struct PublisherLimits {
    max_resident_pages: usize,
    max_unstarted_age: Duration,
}

struct PublisherShared {
    state: Mutex<PublisherState>,
    wake: Condvar,
    interrupt_text_wait: AtomicBool,
    transport: Arc<dyn ChatboxTransport>,
    pacer: ChatboxPacer,
    generation: RuntimeGeneration,
    reporter: PublisherReporter,
    limits: PublisherLimits,
}

struct PublisherJoinState {
    worker: Option<JoinHandle<AppResult<()>>>,
    failure: Option<String>,
}

struct PublisherState {
    lifecycle: PublisherLifecycle,
    units: VecDeque<CompletedUnit>,
    resident_pages: usize,
    next_sequence: u64,
    active_units: HashSet<String>,
    typing_desired: bool,
    typing_epoch: u64,
    typing_attempted_epoch: Option<u64>,
    next_typing_reassert_at: Option<Instant>,
    diagnostics: VecDeque<PublisherDiagnostic>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PublisherLifecycle {
    Running,
    Closing {
        reason: PublisherCloseReason,
        cleanup_attempted: bool,
    },
    Closed,
}

struct CompletedUnit {
    sequence: u64,
    unit_id: String,
    pages: Vec<String>,
    next_page: usize,
    started: bool,
    accepted_at: Instant,
    sent_pages: usize,
    sent_bytes: usize,
    target: Option<String>,
}

impl CompletedUnit {
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
    Diagnostic(PublisherDiagnostic),
    Page {
        sequence: u64,
        page_index: usize,
        text: String,
    },
    Exit,
}

impl CompletedChatboxPublisher {
    pub(crate) fn start(
        transport: Arc<dyn ChatboxTransport>,
        pacer: ChatboxPacer,
        generation: RuntimeGeneration,
        reporter: PublisherReporter,
    ) -> AppResult<Self> {
        Self::start_with_limits(
            transport,
            pacer,
            generation,
            reporter,
            PublisherLimits {
                max_resident_pages: PROVISIONAL_MAX_RESIDENT_PAGES,
                max_unstarted_age: PROVISIONAL_MAX_UNSTARTED_AGE,
            },
        )
    }

    fn start_with_limits(
        transport: Arc<dyn ChatboxTransport>,
        pacer: ChatboxPacer,
        generation: RuntimeGeneration,
        reporter: PublisherReporter,
        limits: PublisherLimits,
    ) -> AppResult<Self> {
        if limits.max_resident_pages == 0 || limits.max_unstarted_age.is_zero() {
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
                active_units: HashSet::new(),
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
            pacer,
            generation,
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
            join_state: Arc::new(Mutex::new(PublisherJoinState {
                worker: Some(worker),
                failure: None,
            })),
        })
    }

    /// Submits one complete lifecycle event without waiting for pacing or OSC.
    pub(crate) fn try_submit(
        &self,
        event: CompletedPublisherEvent,
    ) -> AppResult<PublisherSubmitOutcome> {
        match event {
            CompletedPublisherEvent::Started { unit_id } => {
                let mut state = self.lock_state()?;
                if state.lifecycle != PublisherLifecycle::Running
                    || self.shared.generation.is_hard_stopped()
                {
                    return Ok(PublisherSubmitOutcome::Closed);
                }

                if state.active_units.insert(unit_id) {
                    refresh_typing_desired(&mut state);
                    self.signal_worker_locked();
                }
            }
            CompletedPublisherEvent::Aborted { unit_id } => {
                let mut state = self.lock_state()?;
                if state.lifecycle != PublisherLifecycle::Running
                    || self.shared.generation.is_hard_stopped()
                {
                    return Ok(PublisherSubmitOutcome::Closed);
                }

                resolve_activity(&mut state, &unit_id);
                self.signal_worker_locked();
            }
            CompletedPublisherEvent::Completed { unit_id, text } => {
                return self.try_submit_completed(unit_id, text);
            }
        }

        Ok(PublisherSubmitOutcome::Handled)
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
        let mut join_state = self
            .join_state
            .lock()
            .map_err(|_| AppError::state("Completed publisher join lock was poisoned."))?;

        if let Some(worker) = join_state.worker.take() {
            let result = worker.join().map_err(|_| {
                AppError::runtime("Completed publisher worker thread panicked while stopping.")
            });
            let result = match result {
                Ok(worker_result) => worker_result,
                Err(error) => Err(error),
            };

            if let Err(error) = result {
                join_state.failure = Some(error.to_string());
            }
        }

        match &join_state.failure {
            Some(failure) => Err(AppError::runtime(format!(
                "Completed publisher worker failed: {failure}"
            ))),
            None => Ok(()),
        }
    }

    fn try_submit_completed(
        &self,
        unit_id: String,
        text: String,
    ) -> AppResult<PublisherSubmitOutcome> {
        let pages = match paginate_completed(&text) {
            Ok(pages) => pages,
            Err(error) => {
                let mut state = self.lock_state()?;
                if state.lifecycle != PublisherLifecycle::Running
                    || self.shared.generation.is_hard_stopped()
                {
                    return Ok(PublisherSubmitOutcome::Closed);
                }
                resolve_activity(&mut state, &unit_id);
                state
                    .diagnostics
                    .push_back(PublisherDiagnostic::LayoutFailed {
                        unit_id,
                        reason: describe_layout_error(error),
                    });
                self.signal_worker_locked();
                return Ok(PublisherSubmitOutcome::Handled);
            }
        };

        let mut state = self.lock_state()?;
        if state.lifecycle != PublisherLifecycle::Running
            || self.shared.generation.is_hard_stopped()
        {
            return Ok(PublisherSubmitOutcome::Closed);
        }

        if pages.is_empty() {
            resolve_activity(&mut state, &unit_id);
            self.signal_worker_locked();
            return Ok(PublisherSubmitOutcome::Handled);
        }

        let now = self.shared.pacer.now();
        expire_unstarted_units(&mut state, now, self.shared.limits.max_unstarted_age)?;
        let page_count = pages.len();
        let protected_pages = state
            .units
            .front()
            .filter(|unit| unit.started)
            .map(CompletedUnit::remaining_pages)
            .unwrap_or(0);

        if page_count > self.shared.limits.max_resident_pages
            || protected_pages.saturating_add(page_count) > self.shared.limits.max_resident_pages
        {
            resolve_activity(&mut state, &unit_id);
            state
                .diagnostics
                .push_back(PublisherDiagnostic::UnitRejectedOverload {
                    unit_id,
                    page_count,
                });
            self.signal_worker_locked();
            return Ok(PublisherSubmitOutcome::Handled);
        }

        while state.resident_pages.saturating_add(page_count)
            > self.shared.limits.max_resident_pages
        {
            let Some(position) = state.units.iter().position(|unit| !unit.started) else {
                resolve_activity(&mut state, &unit_id);
                state
                    .diagnostics
                    .push_back(PublisherDiagnostic::UnitRejectedOverload {
                        unit_id,
                        page_count,
                    });
                self.signal_worker_locked();
                return Ok(PublisherSubmitOutcome::Handled);
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
            resolve_activity(&mut state, &dropped.unit_id);
            state
                .diagnostics
                .push_back(PublisherDiagnostic::UnitDroppedOverload {
                    unit_id: dropped.unit_id,
                    page_count: dropped_pages,
                });
        }

        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        state.resident_pages += page_count;
        state.units.push_back(CompletedUnit {
            sequence,
            unit_id,
            pages,
            next_page: 0,
            started: false,
            accepted_at: now,
            sent_pages: 0,
            sent_bytes: 0,
            target: None,
        });
        self.signal_worker_locked();

        Ok(PublisherSubmitOutcome::Handled)
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
                    .pacer
                    .wait_for_turn(Some(&shared.interrupt_text_wait))?;
                let Some(permit) = permit else {
                    continue;
                };
                let mut attempt_result = None;
                let committed = shared.generation.commit_if_active(|| {
                    attempt_result = Some(attempt_selected_page(
                        &shared, sequence, page_index, &text, permit,
                    ));
                })?;

                if committed {
                    if let Some(result) = attempt_result {
                        result?;
                    }
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
    diagnostics.push(PublisherDiagnostic::WorkerFailed {
        reason: failure_reason,
    });
    drop(state);

    if !cleanup_already_attempted && let Err(error) = shared.transport.send_typing(false) {
        diagnostics.push(PublisherDiagnostic::TypingFailed {
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

        expire_unstarted_units(
            &mut state,
            shared.pacer.now(),
            shared.limits.max_unstarted_age,
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
                .is_some_and(|deadline| shared.pacer.now() >= deadline)
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
            let remaining = deadline.saturating_duration_since(shared.pacer.now());
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
    let mut transport_result = None;
    let mut state_error = None;
    let committed = shared.generation.commit_if_active(|| {
        let should_attempt = match shared.state.lock() {
            Ok(state) => {
                state.lifecycle == PublisherLifecycle::Running
                    && state.typing_epoch == epoch
                    && state.typing_desired == is_typing
            }
            Err(_) => {
                state_error = Some(AppError::state(
                    "Completed publisher state lock was poisoned.",
                ));
                false
            }
        };

        if should_attempt {
            transport_result = Some(shared.transport.send_typing(is_typing));
        }
    })?;

    if let Some(error) = state_error {
        return Err(error);
    }

    if !committed {
        return Ok(());
    }

    let Some(result) = transport_result else {
        return Ok(());
    };
    let attempted_at = shared.pacer.now();
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
            .push_back(PublisherDiagnostic::TypingFailed { is_typing, error });
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
            .push_back(PublisherDiagnostic::TypingFailed {
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
    let started_unit_count = state.units.iter().filter(|unit| unit.started).count();

    state.units.clear();
    state.resident_pages = 0;
    state.active_units.clear();
    state.typing_desired = false;
    state.typing_epoch = state.typing_epoch.wrapping_add(1);
    state.typing_attempted_epoch = None;
    state.next_typing_reassert_at = None;

    if page_count > 0 {
        state
            .diagnostics
            .push_back(PublisherDiagnostic::PagesDiscardedOnClose {
                reason,
                unit_count,
                page_count,
                started_unit_count,
            });
    }
}

fn attempt_selected_page(
    shared: &PublisherShared,
    sequence: u64,
    page_index: usize,
    text: &str,
    permit: ChatboxAttemptPermit<'_>,
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

        if !unit.started
            && shared
                .pacer
                .now()
                .saturating_duration_since(unit.accepted_at)
                >= shared.limits.max_unstarted_age
        {
            let Some(expired) = state.units.pop_front() else {
                return Ok(());
            };
            let expired_pages = expired.remaining_pages();
            state.resident_pages = state
                .resident_pages
                .checked_sub(expired_pages)
                .ok_or_else(|| AppError::state("Completed publisher page count underflowed."))?;
            resolve_activity(&mut state, &expired.unit_id);
            state
                .diagnostics
                .push_back(PublisherDiagnostic::UnitExpired {
                    unit_id: expired.unit_id,
                    page_count: expired_pages,
                });
            shared.wake.notify_all();
            return Ok(());
        }

        unit.started = true;
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
                resolve_activity(&mut state, &completed.unit_id);
                state
                    .diagnostics
                    .push_back(PublisherDiagnostic::UnitPublished {
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
            resolve_activity(&mut state, &failed.unit_id);
            state
                .diagnostics
                .push_back(PublisherDiagnostic::UnitSendFailed {
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

fn expire_unstarted_units(
    state: &mut PublisherState,
    now: Instant,
    max_age: Duration,
) -> AppResult<()> {
    loop {
        let position = state.units.iter().position(|unit| {
            !unit.started && now.saturating_duration_since(unit.accepted_at) >= max_age
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
        resolve_activity(state, &expired.unit_id);
        state
            .diagnostics
            .push_back(PublisherDiagnostic::UnitExpired {
                unit_id: expired.unit_id,
                page_count: expired_pages,
            });
    }
}

fn resolve_activity(state: &mut PublisherState, unit_id: &str) {
    if state.active_units.remove(unit_id) {
        refresh_typing_desired(state);
    }
}

fn refresh_typing_desired(state: &mut PublisherState) {
    let desired = !state.active_units.is_empty();
    if desired != state.typing_desired {
        state.typing_desired = desired;
        state.typing_epoch = state.typing_epoch.wrapping_add(1);
        state.typing_attempted_epoch = None;
        state.next_typing_reassert_at = None;
    }
}

fn describe_layout_error(error: ChatboxLayoutError) -> String {
    match error {
        ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units } => format!(
            "One grapheme requires {utf16_units} UTF-16 units, exceeding the 144-unit Chatbox input budget."
        ),
    }
}

#[cfg(test)]
#[path = "chatbox_publisher_tests.rs"]
mod tests;
