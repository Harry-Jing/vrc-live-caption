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

        state = shared
            .wake
            .wait(state)
            .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;
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
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Completed publisher state lock was poisoned."))?;

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
mod tests {
    use super::*;
    use crate::chatbox_pacer::Clock;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Condvar, mpsc};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TransportEvent {
        Text(String),
        Typing(bool),
    }

    struct AdvancingClock {
        now: Mutex<Instant>,
    }

    impl AdvancingClock {
        fn new() -> Self {
            Self {
                now: Mutex::new(Instant::now()),
            }
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
            if let Ok(mut now) = self.now.lock() {
                *now += duration;
            }
        }
    }

    struct ControlledClock {
        state: Mutex<ControlledClockState>,
        changed: Condvar,
    }

    struct ControlledClockState {
        now: Instant,
        automatic: bool,
        sleep_calls: usize,
        total_sleep: Duration,
    }

    impl ControlledClock {
        fn new() -> Self {
            Self {
                state: Mutex::new(ControlledClockState {
                    now: Instant::now(),
                    automatic: false,
                    sleep_calls: 0,
                    total_sleep: Duration::ZERO,
                }),
                changed: Condvar::new(),
            }
        }

        fn release_automatic(&self) {
            if let Ok(mut state) = self.state.lock() {
                state.automatic = true;
                self.changed.notify_all();
            }
        }

        fn advance(&self, duration: Duration) {
            if let Ok(mut state) = self.state.lock() {
                state.now += duration;
                self.changed.notify_all();
            }
        }

        fn wait_for_sleep_calls(&self, count: usize) -> AppResult<()> {
            let state = self
                .state
                .lock()
                .map_err(|_| AppError::state("Controlled clock lock was poisoned."))?;
            let (state, timeout) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(1), |state| {
                    state.sleep_calls < count
                })
                .map_err(|_| AppError::state("Controlled clock lock was poisoned."))?;
            if timeout.timed_out() && state.sleep_calls < count {
                return Err(AppError::runtime(format!(
                    "Expected {count} controlled clock sleep call(s), observed {}.",
                    state.sleep_calls
                )));
            }

            Ok(())
        }

        fn total_sleep(&self) -> AppResult<Duration> {
            self.state
                .lock()
                .map(|state| state.total_sleep)
                .map_err(|_| AppError::state("Controlled clock lock was poisoned."))
        }
    }

    impl Clock for ControlledClock {
        fn now(&self) -> Instant {
            self.state
                .lock()
                .map(|state| state.now)
                .unwrap_or_else(|poisoned| poisoned.into_inner().now)
        }

        fn sleep(&self, duration: Duration) {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.sleep_calls += 1;
            self.changed.notify_all();
            while !state.automatic {
                let Ok(next_state) = self.changed.wait(state) else {
                    return;
                };
                state = next_state;
            }
            state.now += duration;
            state.total_sleep += duration;
        }
    }

    struct RecordingTransport {
        events: Mutex<Vec<TransportEvent>>,
        changed: Condvar,
    }

    impl RecordingTransport {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                changed: Condvar::new(),
            }
        }

        fn wait_for_events(&self, count: usize) -> AppResult<Vec<TransportEvent>> {
            let events = self
                .events
                .lock()
                .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;
            let (events, timeout) = self
                .changed
                .wait_timeout_while(events, Duration::from_secs(1), |events| {
                    events.len() < count
                })
                .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;

            if timeout.timed_out() && events.len() < count {
                return Err(AppError::runtime(format!(
                    "Expected {count} transport events, received {}.",
                    events.len()
                )));
            }

            Ok(events.clone())
        }

        fn events(&self) -> AppResult<Vec<TransportEvent>> {
            self.events
                .lock()
                .map(|events| events.clone())
                .map_err(|_| AppError::state("Recording transport lock was poisoned."))
        }

        fn record(&self, event: TransportEvent) -> AppResult<()> {
            let mut events = self
                .events
                .lock()
                .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;
            events.push(event);
            self.changed.notify_all();
            Ok(())
        }
    }

    impl ChatboxTransport for RecordingTransport {
        fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
            self.record(TransportEvent::Text(text.to_string()))?;
            Ok(ChatboxSendReceipt {
                target: "recording".to_string(),
                byte_count: text.len(),
            })
        }

        fn send_typing(&self, is_typing: bool) -> AppResult<()> {
            self.record(TransportEvent::Typing(is_typing))
        }
    }

    struct BlockFirstTextTransport {
        recording: RecordingTransport,
        should_block: AtomicBool,
        entered: Mutex<Option<mpsc::Sender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl BlockFirstTextTransport {
        fn new(entered: mpsc::Sender<()>, release: mpsc::Receiver<()>) -> Self {
            Self {
                recording: RecordingTransport::new(),
                should_block: AtomicBool::new(true),
                entered: Mutex::new(Some(entered)),
                release: Mutex::new(release),
            }
        }

        fn wait_for_events(&self, count: usize) -> AppResult<Vec<TransportEvent>> {
            self.recording.wait_for_events(count)
        }
    }

    impl ChatboxTransport for BlockFirstTextTransport {
        fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
            self.recording
                .record(TransportEvent::Text(text.to_string()))?;

            if self.should_block.swap(false, Ordering::SeqCst) {
                if let Ok(mut entered) = self.entered.lock()
                    && let Some(entered) = entered.take()
                {
                    let _ = entered.send(());
                }
                self.release
                    .lock()
                    .map_err(|_| AppError::state("Blocking transport lock was poisoned."))?
                    .recv()
                    .map_err(|_| AppError::runtime("Blocking transport was not released."))?;
            }

            Ok(ChatboxSendReceipt {
                target: "blocking".to_string(),
                byte_count: text.len(),
            })
        }

        fn send_typing(&self, is_typing: bool) -> AppResult<()> {
            self.recording.record(TransportEvent::Typing(is_typing))
        }
    }

    #[derive(Clone, Debug)]
    struct TimedTransportEvent {
        at: Instant,
        event: TransportEvent,
    }

    struct ScriptedTransport {
        clock: Arc<dyn Clock>,
        failed_text_attempts: HashSet<usize>,
        failed_typing_attempts: HashSet<usize>,
        next_text_attempt: AtomicUsize,
        next_typing_attempt: AtomicUsize,
        events: Mutex<Vec<TimedTransportEvent>>,
        changed: Condvar,
    }

    impl ScriptedTransport {
        fn new(
            clock: Arc<dyn Clock>,
            failed_text_attempts: impl IntoIterator<Item = usize>,
        ) -> Self {
            Self::with_failures(clock, failed_text_attempts, [])
        }

        fn with_failures(
            clock: Arc<dyn Clock>,
            failed_text_attempts: impl IntoIterator<Item = usize>,
            failed_typing_attempts: impl IntoIterator<Item = usize>,
        ) -> Self {
            Self {
                clock,
                failed_text_attempts: failed_text_attempts.into_iter().collect(),
                failed_typing_attempts: failed_typing_attempts.into_iter().collect(),
                next_text_attempt: AtomicUsize::new(1),
                next_typing_attempt: AtomicUsize::new(1),
                events: Mutex::new(Vec::new()),
                changed: Condvar::new(),
            }
        }

        fn wait_for_events(&self, count: usize) -> AppResult<Vec<TimedTransportEvent>> {
            let events = self
                .events
                .lock()
                .map_err(|_| AppError::state("Scripted transport lock was poisoned."))?;
            let (events, timeout) = self
                .changed
                .wait_timeout_while(events, Duration::from_secs(1), |events| {
                    events.len() < count
                })
                .map_err(|_| AppError::state("Scripted transport lock was poisoned."))?;

            if timeout.timed_out() && events.len() < count {
                return Err(AppError::runtime(format!(
                    "Expected {count} scripted transport events, received {}.",
                    events.len()
                )));
            }

            Ok(events.clone())
        }

        fn record(&self, event: TransportEvent) -> AppResult<()> {
            let mut events = self
                .events
                .lock()
                .map_err(|_| AppError::state("Scripted transport lock was poisoned."))?;
            events.push(TimedTransportEvent {
                at: self.clock.now(),
                event,
            });
            self.changed.notify_all();
            Ok(())
        }
    }

    impl ChatboxTransport for ScriptedTransport {
        fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
            let attempt = self.next_text_attempt.fetch_add(1, Ordering::SeqCst);
            self.record(TransportEvent::Text(text.to_string()))?;
            if self.failed_text_attempts.contains(&attempt) {
                return Err(AppError::osc_send(
                    "scripted",
                    format!("Scripted failure for text attempt {attempt}."),
                ));
            }

            Ok(ChatboxSendReceipt {
                target: "scripted".to_string(),
                byte_count: text.len(),
            })
        }

        fn send_typing(&self, is_typing: bool) -> AppResult<()> {
            let attempt = self.next_typing_attempt.fetch_add(1, Ordering::SeqCst);
            self.record(TransportEvent::Typing(is_typing))?;
            if self.failed_typing_attempts.contains(&attempt) {
                return Err(AppError::osc_send(
                    "scripted",
                    format!("Scripted failure for typing attempt {attempt}."),
                ));
            }

            Ok(())
        }
    }

    fn recording_reporter() -> (PublisherReporter, Arc<Mutex<Vec<PublisherDiagnostic>>>) {
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let recorded_diagnostics = Arc::clone(&diagnostics);
        let reporter: PublisherReporter = Arc::new(move |diagnostic| {
            if let Ok(mut diagnostics) = recorded_diagnostics.lock() {
                diagnostics.push(diagnostic);
            }
        });

        (reporter, diagnostics)
    }

    fn submit_handled(
        publisher: &CompletedChatboxPublisher,
        event: CompletedPublisherEvent,
    ) -> AppResult<()> {
        assert_eq!(
            publisher.try_submit(event)?,
            PublisherSubmitOutcome::Handled
        );
        Ok(())
    }

    #[test]
    fn publishes_every_exact_page_in_order() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let clock = Arc::new(AdvancingClock::new());
        let pacer = ChatboxPacer::with_clock(clock);
        let generation = RuntimeGeneration::active();
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let recorded_diagnostics = Arc::clone(&diagnostics);
        let reporter: PublisherReporter = Arc::new(move |diagnostic| {
            if let Ok(mut diagnostics) = recorded_diagnostics.lock() {
                diagnostics.push(diagnostic);
            }
        });
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            pacer,
            generation,
            reporter,
            PublisherLimits {
                max_resident_pages: 8,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;
        let text = "中".repeat(136);
        let expected_pages = paginate_completed(&text)
            .map_err(|error| AppError::runtime(describe_layout_error(error)))?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "unit-a".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "unit-a".to_string(),
                text,
            },
        )?;

        let events = transport.wait_for_events(expected_pages.len() + 2)?;
        let sent_pages = events
            .iter()
            .filter_map(|event| match event {
                TransportEvent::Text(text) => Some(text.clone()),
                TransportEvent::Typing(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sent_pages, expected_pages);
        assert_eq!(events.first(), Some(&TransportEvent::Typing(true)));
        assert_eq!(events.last(), Some(&TransportEvent::Typing(false)));

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;

        Ok(())
    }

    #[test]
    fn submission_does_not_wait_for_an_in_flight_osc_attempt() -> AppResult<()> {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let transport = Arc::new(BlockFirstTextTransport::new(
            entered_sender,
            release_receiver,
        ));
        let clock = Arc::new(AdvancingClock::new());
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(clock),
            RuntimeGeneration::active(),
            Arc::new(|_| {}),
            PublisherLimits {
                max_resident_pages: 8,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "unit-a".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "unit-a".to_string(),
                text: "first".to_string(),
            },
        )?;
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("First OSC attempt did not start."))?;

        let submitted_publisher = publisher.clone();
        let (submitted_sender, submitted_receiver) = mpsc::channel();
        let submitter = thread::spawn(move || -> AppResult<()> {
            submit_handled(
                &submitted_publisher,
                CompletedPublisherEvent::Started {
                    unit_id: "unit-b".to_string(),
                },
            )?;
            submit_handled(
                &submitted_publisher,
                CompletedPublisherEvent::Completed {
                    unit_id: "unit-b".to_string(),
                    text: "second".to_string(),
                },
            )?;
            let _ = submitted_sender.send(());
            Ok(())
        });

        submitted_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Publisher submission waited for OSC."))?;
        release_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the OSC attempt."))?;
        submitter
            .join()
            .map_err(|_| AppError::runtime("Publisher submitter panicked."))??;

        let events = transport.wait_for_events(4)?;
        assert_eq!(
            events,
            vec![
                TransportEvent::Typing(true),
                TransportEvent::Text("first".to_string()),
                TransportEvent::Text("second".to_string()),
                TransportEvent::Typing(false),
            ]
        );

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;

        Ok(())
    }

    #[test]
    fn overload_drops_only_the_oldest_whole_unstarted_unit() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let clock = Arc::new(ControlledClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
            .attempt(|| Ok(()))?;
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            pacer,
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 3,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        for (unit_id, text) in [
            ("unit-a", "中".repeat(136)),
            ("unit-b", "B".to_string()),
            ("unit-c", "中".repeat(136)),
        ] {
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Started {
                    unit_id: unit_id.to_string(),
                },
            )?;
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Completed {
                    unit_id: unit_id.to_string(),
                    text,
                },
            )?;
        }

        clock.release_automatic();
        let events = transport.wait_for_events(5)?;
        let sent_pages = events
            .iter()
            .filter_map(|event| match event {
                TransportEvent::Text(text) => Some(text.clone()),
                TransportEvent::Typing(_) => None,
            })
            .collect::<Vec<_>>();
        let mut expected_pages = vec!["B".to_string()];
        expected_pages.extend(
            paginate_completed(&"中".repeat(136))
                .map_err(|error| AppError::runtime(describe_layout_error(error)))?,
        );
        assert_eq!(sent_pages, expected_pages);
        assert_eq!(events.first(), Some(&TransportEvent::Typing(true)));
        assert_eq!(events.last(), Some(&TransportEvent::Typing(false)));

        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::UnitDroppedOverload {
                unit_id,
                page_count: 2,
            } if unit_id == "unit-a"
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;

        Ok(())
    }

    #[test]
    fn failed_page_consumes_pacing_and_aborts_the_rest_of_its_unit() -> AppResult<()> {
        let clock = Arc::new(ControlledClock::new());
        let clock_for_transport: Arc<dyn Clock> = clock.clone();
        let transport = Arc::new(ScriptedTransport::new(clock_for_transport, [2]));
        let pacer = ChatboxPacer::with_clock(clock.clone());
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
            .attempt(|| Ok(()))?;
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            pacer,
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 8,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;
        let first_text = "中".repeat(271);
        let first_pages = paginate_completed(&first_text)
            .map_err(|error| AppError::runtime(describe_layout_error(error)))?;
        assert_eq!(first_pages.len(), 3);

        for (unit_id, text) in [("unit-a", first_text), ("unit-b", "B".to_string())] {
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Started {
                    unit_id: unit_id.to_string(),
                },
            )?;
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Completed {
                    unit_id: unit_id.to_string(),
                    text,
                },
            )?;
        }
        clock.release_automatic();

        let events = transport.wait_for_events(5)?;
        let text_attempts = events
            .iter()
            .filter(|event| matches!(event.event, TransportEvent::Text(_)))
            .collect::<Vec<_>>();
        assert_eq!(text_attempts.len(), 3);
        assert_eq!(
            text_attempts
                .iter()
                .map(|event| match &event.event {
                    TransportEvent::Text(text) => text.clone(),
                    TransportEvent::Typing(_) => String::new(),
                })
                .collect::<Vec<_>>(),
            vec![
                first_pages[0].clone(),
                first_pages[1].clone(),
                "B".to_string()
            ]
        );
        assert_eq!(
            text_attempts[1]
                .at
                .saturating_duration_since(text_attempts[0].at),
            Duration::from_secs(1)
        );
        assert_eq!(
            text_attempts[2]
                .at
                .saturating_duration_since(text_attempts[1].at),
            Duration::from_secs(1)
        );

        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::UnitSendFailed {
                unit_id,
                page_index: 2,
                page_count: 3,
                pages_sent: 1,
                ..
            } if unit_id == "unit-a"
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;

        Ok(())
    }

    #[test]
    fn started_unit_is_protected_and_new_unit_is_rejected_without_evicting_others() -> AppResult<()>
    {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let transport = Arc::new(BlockFirstTextTransport::new(
            entered_sender,
            release_receiver,
        ));
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(Arc::new(AdvancingClock::new())),
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 3,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;
        let first_text = "中".repeat(136);
        let first_pages = paginate_completed(&first_text)
            .map_err(|error| AppError::runtime(describe_layout_error(error)))?;
        assert_eq!(first_pages.len(), 2);

        for (unit_id, text) in [("unit-a", first_text), ("unit-b", "B".to_string())] {
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Started {
                    unit_id: unit_id.to_string(),
                },
            )?;
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Completed {
                    unit_id: unit_id.to_string(),
                    text,
                },
            )?;
        }
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Started unit did not enter transport."))?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "unit-c".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "unit-c".to_string(),
                text: "中".repeat(136),
            },
        )?;
        release_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the started unit."))?;

        let events = transport.wait_for_events(5)?;
        let sent_pages = events
            .iter()
            .filter_map(|event| match event {
                TransportEvent::Text(text) => Some(text.clone()),
                TransportEvent::Typing(_) => None,
            })
            .collect::<Vec<_>>();
        let mut expected_pages = first_pages;
        expected_pages.push("B".to_string());
        assert_eq!(sent_pages, expected_pages);

        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::UnitRejectedOverload {
                unit_id,
                page_count: 2,
            } if unit_id == "unit-c"
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;
        Ok(())
    }

    #[test]
    fn unit_larger_than_capacity_is_rejected_whole_without_changing_the_queue() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let clock = Arc::new(ControlledClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
            .attempt(|| Ok(()))?;
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            pacer,
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 2,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        for (unit_id, text) in [("kept", "A".to_string()), ("oversized", "中".repeat(271))] {
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Started {
                    unit_id: unit_id.to_string(),
                },
            )?;
            submit_handled(
                &publisher,
                CompletedPublisherEvent::Completed {
                    unit_id: unit_id.to_string(),
                    text,
                },
            )?;
        }
        clock.release_automatic();

        assert_eq!(
            transport.wait_for_events(3)?,
            vec![
                TransportEvent::Typing(true),
                TransportEvent::Text("A".to_string()),
                TransportEvent::Typing(false),
            ]
        );
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::UnitRejectedOverload {
                unit_id,
                page_count: 3,
            } if unit_id == "oversized"
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;
        Ok(())
    }

    #[test]
    fn stale_unstarted_unit_expires_as_one_complete_publication() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let clock = Arc::new(ControlledClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
            .attempt(|| Ok(()))?;
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            pacer,
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "expired".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "expired".to_string(),
                text: "中".repeat(136),
            },
        )?;
        transport.wait_for_events(1)?;
        clock.wait_for_sleep_calls(1)?;
        clock.advance(Duration::from_secs(30));
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "fresh".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "fresh".to_string(),
                text: "fresh".to_string(),
            },
        )?;
        clock.release_automatic();

        assert_eq!(
            transport.wait_for_events(3)?,
            vec![
                TransportEvent::Typing(true),
                TransportEvent::Text("fresh".to_string()),
                TransportEvent::Typing(false),
            ]
        );
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::UnitExpired {
                unit_id,
                page_count: 2,
            } if unit_id == "expired"
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;
        Ok(())
    }

    #[test]
    fn overlapping_activity_keeps_typing_on_until_the_last_unit_resolves() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(Arc::new(AdvancingClock::new())),
            RuntimeGeneration::active(),
            Arc::new(|_| {}),
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "unit-a".to_string(),
            },
        )?;
        transport.wait_for_events(1)?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "unit-b".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Aborted {
                unit_id: "unit-a".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "unit-b".to_string(),
                text: "B".to_string(),
            },
        )?;

        assert_eq!(
            transport.wait_for_events(3)?,
            vec![
                TransportEvent::Typing(true),
                TransportEvent::Text("B".to_string()),
                TransportEvent::Typing(false),
            ]
        );
        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;
        Ok(())
    }

    #[test]
    fn layout_failure_resolves_typing_without_attempting_text() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(Arc::new(AdvancingClock::new())),
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "layout-failure".to_string(),
            },
        )?;
        transport.wait_for_events(1)?;
        let oversized_grapheme = format!("a{}", "\u{301}".repeat(144));
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "layout-failure".to_string(),
                text: oversized_grapheme,
            },
        )?;

        assert_eq!(
            transport.wait_for_events(2)?,
            vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
        );
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::LayoutFailed { unit_id, .. }
                if unit_id == "layout-failure"
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;
        Ok(())
    }

    #[test]
    fn failed_typing_on_is_diagnosed_and_still_followed_by_typing_off() -> AppResult<()> {
        let clock = Arc::new(AdvancingClock::new());
        let transport_clock: Arc<dyn Clock> = clock.clone();
        let transport = Arc::new(ScriptedTransport::with_failures(transport_clock, [], [1]));
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(clock),
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "typing-failure".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "typing-failure".to_string(),
                text: "caption".to_string(),
            },
        )?;

        let events = transport.wait_for_events(3)?;
        assert_eq!(
            events
                .iter()
                .map(|event| event.event.clone())
                .collect::<Vec<_>>(),
            vec![
                TransportEvent::Typing(true),
                TransportEvent::Text("caption".to_string()),
                TransportEvent::Typing(false),
            ]
        );
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::TypingFailed {
                is_typing: true,
                ..
            }
        )));
        drop(diagnostics);

        publisher.request_close(PublisherCloseReason::Stop)?;
        publisher.join()?;
        Ok(())
    }

    #[test]
    fn stop_interrupts_a_pacing_wait_discards_late_submissions_and_cleans_typing_once()
    -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let clock = Arc::new(ControlledClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
            .attempt(|| Ok(()))?;
        let generation = RuntimeGeneration::active();
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            pacer,
            generation.clone(),
            reporter,
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "stopped".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "stopped".to_string(),
                text: "must not send".to_string(),
            },
        )?;
        transport.wait_for_events(1)?;
        clock.wait_for_sleep_calls(1)?;

        generation.request_stop(Some(&publisher))?;
        assert_eq!(
            publisher.try_submit(CompletedPublisherEvent::Completed {
                unit_id: "late".to_string(),
                text: "late".to_string(),
            })?,
            PublisherSubmitOutcome::Closed
        );
        clock.release_automatic();
        publisher.join()?;
        generation.request_stop(Some(&publisher))?;
        publisher.join()?;

        assert_eq!(
            transport.events()?,
            vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
        );
        assert_eq!(clock.total_sleep()?, Duration::from_millis(100));
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::PagesDiscardedOnClose {
                reason: PublisherCloseReason::Stop,
                unit_count: 1,
                page_count: 1,
                started_unit_count: 0,
            }
        )));

        Ok(())
    }

    #[test]
    fn stop_waits_for_a_linearized_attempt_then_discards_every_remaining_page() -> AppResult<()> {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let transport = Arc::new(BlockFirstTextTransport::new(
            entered_sender,
            release_receiver,
        ));
        let generation = RuntimeGeneration::active();
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(Arc::new(AdvancingClock::new())),
            generation.clone(),
            reporter,
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;
        let pages = paginate_completed(&"中".repeat(136))
            .map_err(|error| AppError::runtime(describe_layout_error(error)))?;

        submit_handled(
            &publisher,
            CompletedPublisherEvent::Started {
                unit_id: "in-flight".to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            CompletedPublisherEvent::Completed {
                unit_id: "in-flight".to_string(),
                text: "中".repeat(136),
            },
        )?;
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("OSC attempt did not reach transport."))?;

        let stop_generation = generation.clone();
        let stop_publisher = publisher.clone();
        let (stop_started_sender, stop_started_receiver) = mpsc::channel();
        let (stop_finished_sender, stop_finished_receiver) = mpsc::channel();
        let stop = thread::spawn(move || -> AppResult<()> {
            stop_started_sender
                .send(())
                .map_err(|_| AppError::runtime("Could not announce Stop."))?;
            let result = stop_generation.request_stop(Some(&stop_publisher));
            let _ = stop_finished_sender.send(());
            result
        });
        stop_started_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Stop test thread did not start."))?;
        assert!(matches!(
            stop_finished_receiver.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert_eq!(
            publisher.try_submit(CompletedPublisherEvent::Completed {
                unit_id: "late".to_string(),
                text: "late".to_string(),
            })?,
            PublisherSubmitOutcome::Closed
        );

        release_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the OSC attempt."))?;
        stop_finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Stop did not wait for the OSC attempt."))?;
        stop.join()
            .map_err(|_| AppError::runtime("Stop test thread panicked."))??;
        publisher.join()?;

        let events = transport.wait_for_events(3)?;
        assert_eq!(
            events,
            vec![
                TransportEvent::Typing(true),
                TransportEvent::Text(pages[0].clone()),
                TransportEvent::Typing(false),
            ]
        );
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::PagesDiscardedOnClose {
                reason: PublisherCloseReason::Stop,
                page_count: 1,
                started_unit_count: 1,
                ..
            }
        )));

        Ok(())
    }

    #[test]
    fn concurrent_close_and_join_perform_one_cleanup() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(Arc::new(AdvancingClock::new())),
            RuntimeGeneration::active(),
            Arc::new(|_| {}),
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut closers = Vec::new();

        for _ in 0..2 {
            let closer = publisher.clone();
            let closer_barrier = barrier.clone();
            closers.push(thread::spawn(move || -> AppResult<()> {
                closer_barrier.wait();
                closer.request_close(PublisherCloseReason::Stop)?;
                closer.join()
            }));
        }
        barrier.wait();
        for closer in closers {
            closer
                .join()
                .map_err(|_| AppError::runtime("Concurrent closer panicked."))??;
        }

        assert_eq!(transport.events()?, vec![TransportEvent::Typing(false)]);
        Ok(())
    }

    #[test]
    fn poisoned_state_still_wakes_the_worker_and_attempts_one_cleanup() -> AppResult<()> {
        let transport = Arc::new(RecordingTransport::new());
        let (reporter, diagnostics) = recording_reporter();
        let publisher = CompletedChatboxPublisher::start_with_limits(
            transport.clone(),
            ChatboxPacer::with_clock(Arc::new(AdvancingClock::new())),
            RuntimeGeneration::active(),
            reporter,
            PublisherLimits {
                max_resident_pages: 4,
                max_unstarted_age: Duration::from_secs(30),
            },
        )?;
        let shared = Arc::clone(&publisher.shared);
        let poisoner = thread::spawn(move || {
            if let Ok(_state) = shared.state.lock() {
                std::panic::resume_unwind(Box::new("poison publisher state for shutdown coverage"));
            }
        });
        assert!(poisoner.join().is_err());

        assert!(publisher.request_close(PublisherCloseReason::Stop).is_err());
        assert!(publisher.join().is_err());
        assert_eq!(transport.events()?, vec![TransportEvent::Typing(false)]);
        let diagnostics = diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic, PublisherDiagnostic::WorkerFailed { .. }))
        );

        Ok(())
    }
}
