//! Latest-wins Live publication for the VRChat Chatbox.
//!
//! The publisher observes backend-authoritative caption-session aggregates. It
//! never reconstructs provider deltas and never queues historical Live
//! revisions. One worker owns observation timing, process-wide pacing, OSC
//! attempts, and the Stop cleanup; producers only replace in-memory state.

use super::common::{
    PublisherCloseReason, PublisherLifecycle, PublisherSubmitOutcome, PublisherWorkerJoin,
    TYPING_REASSERT_INTERVAL, describe_layout_error,
};
use super::layout::render_live_viewport;
use super::pacer::ChatboxPacer;
use super::transport::{ChatboxSendReceipt, ChatboxTransport};
use crate::capability_planner::ResolvedPublicationPolicy;
use crate::caption_session::{
    CaptionLane, CaptionSessionSnapshotV1, CaptionSnapshotV1, CaptionState,
};
use crate::error::{AppError, AppResult};
use crate::runtime_generation::RuntimeGeneration;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const OBSERVATION_WAIT_POLL: Duration = Duration::from_millis(50);

pub(crate) type LivePublisherReporter = Arc<dyn Fn(LivePublisherDiagnostic) + Send + Sync>;

#[derive(Debug)]
pub(crate) enum LivePublisherDiagnostic {
    ViewPublished {
        stream_id: String,
        unit_id: Option<String>,
        revision: u64,
        byte_count: usize,
        target: String,
    },
    ViewSendFailed {
        stream_id: String,
        unit_id: Option<String>,
        revision: u64,
        error: AppError,
    },
    LayoutFailed {
        stream_id: String,
        unit_id: Option<String>,
        revision: u64,
        reason: String,
    },
    DraftDiscardedOnClose {
        reason: PublisherCloseReason,
    },
    TypingFailed {
        error: AppError,
    },
    WorkerFailed {
        reason: String,
    },
}

#[derive(Clone)]
pub(crate) struct LiveChatboxPublisher {
    shared: Arc<LivePublisherShared>,
    worker_join: PublisherWorkerJoin,
}

struct LivePublisherShared {
    state: Mutex<LivePublisherState>,
    wake: Condvar,
    interrupt_text_wait: AtomicBool,
    output_gate: Mutex<()>,
    transport: Arc<dyn ChatboxTransport>,
    pacer: ChatboxPacer,
    generation_id: u64,
    generation: RuntimeGeneration,
    reporter: LivePublisherReporter,
    policy: LiveObservationPolicy,
}

#[derive(Clone, Copy)]
enum LiveObservationPolicy {
    Unit { observation_window: Duration },
}

struct LivePublisherState {
    lifecycle: PublisherLifecycle,
    highest_snapshot_revision: u64,
    stream_id: Option<String>,
    unit_first_seen: HashMap<String, Instant>,
    candidate: Option<LiveCandidate>,
    last_attempted: Option<LiveCandidateAttempt>,
    last_published: Option<PublishedLiveView>,
    last_layout_failure: Option<LiveCandidateIdentity>,
    typing_desired: bool,
    typing_epoch: u64,
    typing_attempted_epoch: Option<u64>,
    next_typing_reassert_at: Option<Instant>,
    diagnostics: VecDeque<LivePublisherDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveScope {
    stream_id: String,
    unit_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveCandidateIdentity {
    scope: LiveScope,
    revision: u64,
    state: CaptionState,
}

#[derive(Clone)]
struct LiveCandidate {
    identity: LiveCandidateIdentity,
    view: String,
    ready_at: Instant,
}

struct LiveCandidateAttempt {
    identity: LiveCandidateIdentity,
    view: String,
}

impl LiveCandidateAttempt {
    fn matches(&self, candidate: &LiveCandidate) -> bool {
        self.identity == candidate.identity && self.view == candidate.view
    }
}

struct PublishedLiveView {
    scope: LiveScope,
    view: String,
}

enum LiveWorkerItem {
    Candidate(LiveCandidate),
    Typing { epoch: u64, is_typing: bool },
    Diagnostic(LivePublisherDiagnostic),
    CleanupTyping,
    Exit,
}

impl LiveChatboxPublisher {
    pub(crate) fn start(
        transport: Arc<dyn ChatboxTransport>,
        pacer: ChatboxPacer,
        generation: RuntimeGeneration,
        policy: ResolvedPublicationPolicy,
        reporter: LivePublisherReporter,
    ) -> AppResult<Self> {
        let generation_id = generation.generation_id();
        if generation_id == 0 {
            return Err(AppError::state(
                "Live publisher generation must be greater than zero.",
            ));
        }

        let policy = match policy {
            ResolvedPublicationPolicy::LiveUnit {
                observation_window_ms: 0,
            } => {
                return Err(AppError::state(
                    "Live publisher observation window must be greater than zero.",
                ));
            }
            ResolvedPublicationPolicy::LiveUnit {
                observation_window_ms,
            } => LiveObservationPolicy::Unit {
                observation_window: Duration::from_millis(observation_window_ms),
            },
            ResolvedPublicationPolicy::Completed => {
                return Err(AppError::state(
                    "Live publisher requires a resolved Live publication policy.",
                ));
            }
        };

        let shared = Arc::new(LivePublisherShared {
            state: Mutex::new(LivePublisherState {
                lifecycle: PublisherLifecycle::Running,
                highest_snapshot_revision: 0,
                stream_id: None,
                unit_first_seen: HashMap::new(),
                candidate: None,
                last_attempted: None,
                last_published: None,
                last_layout_failure: None,
                typing_desired: false,
                typing_epoch: 0,
                typing_attempted_epoch: Some(0),
                next_typing_reassert_at: None,
                diagnostics: VecDeque::new(),
            }),
            wake: Condvar::new(),
            interrupt_text_wait: AtomicBool::new(false),
            output_gate: Mutex::new(()),
            transport,
            pacer,
            generation_id,
            generation,
            reporter,
            policy,
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("vrc-live-caption-live-publisher".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_live_worker(Arc::clone(&worker_shared))
                }));
                match result {
                    Ok(worker_result) => {
                        if let Err(error) = &worker_result {
                            emergency_close_after_worker_failure(
                                &worker_shared,
                                format!("Live publisher worker failed: {error}"),
                            );
                        }
                        worker_result
                    }
                    Err(panic) => {
                        emergency_close_after_worker_failure(
                            &worker_shared,
                            "Live publisher worker panicked.".to_string(),
                        );
                        std::panic::resume_unwind(panic);
                    }
                }
            })
            .map_err(|error| {
                AppError::runtime(format!("Failed to start Live publisher worker: {error}"))
            })?;

        Ok(Self {
            shared,
            worker_join: PublisherWorkerJoin::new("Live", worker),
        })
    }

    /// Replaces the observed Live state without waiting for pacing or OSC.
    pub(crate) fn try_observe(
        &self,
        snapshot: &CaptionSessionSnapshotV1,
    ) -> AppResult<PublisherSubmitOutcome> {
        let mut state = self.lock_state()?;
        if state.lifecycle != PublisherLifecycle::Running
            || self.shared.generation.is_hard_stop_requested()
        {
            return Ok(PublisherSubmitOutcome::Closed);
        }

        let Some(active) = snapshot.active.as_ref() else {
            return Ok(PublisherSubmitOutcome::Handled);
        };
        if active.generation != self.shared.generation_id
            || snapshot.snapshot_revision <= state.highest_snapshot_revision
        {
            return Ok(PublisherSubmitOutcome::Handled);
        }

        let now = self.shared.pacer.now();
        if state.stream_id.as_deref() != Some(active.stream_id.as_str()) {
            state.stream_id = Some(active.stream_id.clone());
            state.unit_first_seen.clear();
            state.candidate = None;
            state.last_attempted = None;
            state.last_layout_failure = None;
        }
        state.highest_snapshot_revision = snapshot.snapshot_revision;

        let active_unit_ids = snapshot
            .active_units
            .iter()
            .map(|unit| unit.unit_id.as_str())
            .collect::<Vec<_>>();
        state
            .unit_first_seen
            .retain(|unit_id, _| active_unit_ids.contains(&unit_id.as_str()));
        for unit_id in active_unit_ids {
            state
                .unit_first_seen
                .entry(unit_id.to_string())
                .or_insert(now);
        }

        let source_captions = snapshot
            .captions
            .iter()
            .filter(|caption| {
                caption.generation == self.shared.generation_id
                    && caption.stream_id == active.stream_id
                    && caption.lane == CaptionLane::Source
                    && !caption.text.trim().is_empty()
            })
            .collect::<Vec<_>>();
        let caption = source_captions.first().copied();
        let recent_source_text = compose_recent_source(&source_captions);
        let candidate = match caption {
            Some(caption) => self.candidate_from_captions(
                &mut state,
                caption,
                &source_captions,
                &recent_source_text,
                now,
            ),
            None => None,
        };
        state.candidate = candidate;
        refresh_typing_desired(&mut state);

        self.shared
            .interrupt_text_wait
            .store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        Ok(PublisherSubmitOutcome::Handled)
    }

    /// Closes admission and establishes a local no-later-text boundary.
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
                discard_live_candidate_on_close(&mut state, reason);
                state.lifecycle = PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted: false,
                };
            }
            PublisherLifecycle::Closing {
                reason: current_reason,
                cleanup_attempted,
            } if reason == PublisherCloseReason::Stop
                && current_reason == PublisherCloseReason::RuntimeError =>
            {
                state.lifecycle = PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted,
                };
            }
            PublisherLifecycle::Closing { .. } | PublisherLifecycle::Closed => {}
        }

        // A selected candidate clears this flag before waiting on the pacer.
        // Reassert it under the state lock so Stop cannot lose its wake-up to
        // that selection race.
        self.shared
            .interrupt_text_wait
            .store(true, Ordering::SeqCst);
        let perform_poison_cleanup = if state_was_poisoned {
            match state.lifecycle {
                PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted: false,
                } => {
                    discard_live_candidate_on_close(&mut state, reason);
                    state.lifecycle = PublisherLifecycle::Closing {
                        reason,
                        cleanup_attempted: true,
                    };
                    true
                }
                PublisherLifecycle::Closing {
                    cleanup_attempted: true,
                    ..
                }
                | PublisherLifecycle::Running
                | PublisherLifecycle::Closed => false,
            }
        } else {
            false
        };
        self.shared.wake.notify_all();
        drop(state);

        // The worker holds this only around its final candidate check and OSC
        // call. Producers never take it, so a slow transport cannot block
        // recognition ingestion while request_close still gets a linearizable
        // cutoff before it returns.
        let (_output, output_gate_was_poisoned) = match self.shared.output_gate.lock() {
            Ok(output) => (output, false),
            Err(poisoned) => (poisoned.into_inner(), true),
        };
        let cleanup_note = if perform_poison_cleanup {
            match self.shared.transport.send_typing(false) {
                Ok(()) => " A best-effort typing-off cleanup was attempted.",
                Err(_) => " The best-effort typing-off cleanup also failed.",
            }
        } else {
            ""
        };

        if state_was_poisoned || output_gate_was_poisoned {
            Err(AppError::state(format!(
                "Live publisher synchronization was poisoned while closing; shutdown was still requested.{cleanup_note}"
            )))
        } else {
            Ok(())
        }
    }

    pub(crate) fn join(&self) -> AppResult<()> {
        self.worker_join.join()
    }

    fn candidate_from_captions(
        &self,
        state: &mut LivePublisherState,
        caption: &CaptionSnapshotV1,
        source_captions: &[&CaptionSnapshotV1],
        recent_source_text: &str,
        now: Instant,
    ) -> Option<LiveCandidate> {
        let scope = LiveScope {
            stream_id: caption.stream_id.clone(),
            unit_id: caption.unit_id.clone(),
        };
        let identity = LiveCandidateIdentity {
            scope,
            revision: caption.revision,
            state: caption.state,
        };
        let ready_at = self.candidate_ready_at(state, source_captions, now)?;

        match render_live_viewport(recent_source_text) {
            Ok(view) if !view.is_empty() => Some(LiveCandidate {
                identity,
                view,
                ready_at,
            }),
            Ok(_) => None,
            Err(error) => {
                if state.last_layout_failure.as_ref() != Some(&identity) {
                    state.last_layout_failure = Some(identity.clone());
                    state
                        .diagnostics
                        .push_back(LivePublisherDiagnostic::LayoutFailed {
                            stream_id: identity.scope.stream_id,
                            unit_id: identity.scope.unit_id,
                            revision: identity.revision,
                            reason: describe_layout_error(error),
                        });
                }
                None
            }
        }
    }

    fn candidate_ready_at(
        &self,
        state: &mut LivePublisherState,
        source_captions: &[&CaptionSnapshotV1],
        now: Instant,
    ) -> Option<Instant> {
        let LiveObservationPolicy::Unit { observation_window } = self.shared.policy;
        let mut ready_at = now;
        for caption in source_captions {
            match (caption.unit_id.as_deref(), caption.state) {
                (Some(_), CaptionState::Completed) => {}
                (Some(unit_id), CaptionState::Ongoing) => {
                    let first_seen = *state
                        .unit_first_seen
                        .entry(unit_id.to_string())
                        .or_insert(now);
                    ready_at = ready_at.max(first_seen + observation_window);
                }
                (None, _) => return None,
            }
        }
        Some(ready_at)
    }

    fn lock_state(&self) -> AppResult<std::sync::MutexGuard<'_, LivePublisherState>> {
        self.shared
            .state
            .lock()
            .map_err(|_| AppError::state("Live publisher state lock was poisoned."))
    }
}

fn run_live_worker(shared: Arc<LivePublisherShared>) -> AppResult<()> {
    loop {
        match next_live_worker_item(&shared)? {
            LiveWorkerItem::Candidate(candidate) => process_live_candidate(&shared, candidate)?,
            LiveWorkerItem::Typing { epoch, is_typing } => {
                process_typing(&shared, epoch, is_typing)?;
            }
            LiveWorkerItem::Diagnostic(diagnostic) => (shared.reporter)(diagnostic),
            LiveWorkerItem::CleanupTyping => process_cleanup_typing(&shared)?,
            LiveWorkerItem::Exit => return Ok(()),
        }
    }
}

fn emergency_close_after_worker_failure(shared: &LivePublisherShared, failure_reason: String) {
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
    discard_live_candidate_on_close(&mut state, reason);
    state.lifecycle = PublisherLifecycle::Closed;
    let mut diagnostics = state.diagnostics.drain(..).collect::<Vec<_>>();
    diagnostics.push(LivePublisherDiagnostic::WorkerFailed {
        reason: failure_reason,
    });
    drop(state);

    if !cleanup_already_attempted && let Err(error) = shared.transport.send_typing(false) {
        diagnostics.push(LivePublisherDiagnostic::TypingFailed { error });
    }
    for diagnostic in diagnostics {
        (shared.reporter)(diagnostic);
    }
    shared.wake.notify_all();
}

fn next_live_worker_item(shared: &LivePublisherShared) -> AppResult<LiveWorkerItem> {
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;

    loop {
        match state.lifecycle {
            PublisherLifecycle::Closing {
                reason,
                cleanup_attempted: false,
            } => {
                state.lifecycle = PublisherLifecycle::Closing {
                    reason,
                    cleanup_attempted: true,
                };
                return Ok(LiveWorkerItem::CleanupTyping);
            }
            PublisherLifecycle::Closing {
                cleanup_attempted: true,
                ..
            } => {
                if let Some(diagnostic) = state.diagnostics.pop_front() {
                    return Ok(LiveWorkerItem::Diagnostic(diagnostic));
                }
                state.lifecycle = PublisherLifecycle::Closed;
                shared.wake.notify_all();
                return Ok(LiveWorkerItem::Exit);
            }
            PublisherLifecycle::Closed => return Ok(LiveWorkerItem::Exit),
            PublisherLifecycle::Running => {}
        }

        if state.typing_attempted_epoch != Some(state.typing_epoch) {
            let epoch = state.typing_epoch;
            let is_typing = state.typing_desired;
            state.typing_attempted_epoch = Some(epoch);
            return Ok(LiveWorkerItem::Typing { epoch, is_typing });
        }

        if state.typing_desired
            && state
                .next_typing_reassert_at
                .is_some_and(|deadline| shared.pacer.now() >= deadline)
        {
            return Ok(LiveWorkerItem::Typing {
                epoch: state.typing_epoch,
                is_typing: true,
            });
        }

        if let Some(diagnostic) = state.diagnostics.pop_front() {
            return Ok(LiveWorkerItem::Diagnostic(diagnostic));
        }

        let now = shared.pacer.now();
        let mut next_deadline = state
            .typing_desired
            .then_some(state.next_typing_reassert_at)
            .flatten();
        if let Some(candidate) = state.candidate.as_ref() {
            // The aggregate viewport can change when a non-head caption is
            // removed even though the head caption identity stays unchanged.
            let already_attempted = state
                .last_attempted
                .as_ref()
                .is_some_and(|attempt| attempt.matches(candidate));
            let already_published = state.last_published.as_ref().is_some_and(|published| {
                published.scope == candidate.identity.scope && published.view == candidate.view
            });
            if !already_attempted && !already_published && now >= candidate.ready_at {
                let selected = candidate.clone();
                shared.interrupt_text_wait.store(false, Ordering::SeqCst);
                return Ok(LiveWorkerItem::Candidate(selected));
            }

            if !already_attempted && !already_published && now < candidate.ready_at {
                next_deadline = Some(next_deadline.map_or(candidate.ready_at, |deadline| {
                    deadline.min(candidate.ready_at)
                }));
            }
        }

        if let Some(deadline) = next_deadline {
            let remaining = deadline.saturating_duration_since(shared.pacer.now());
            let (next_state, _) = shared
                .wake
                .wait_timeout(state, remaining.min(OBSERVATION_WAIT_POLL))
                .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;
            state = next_state;
        } else {
            state = shared
                .wake
                .wait(state)
                .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;
        }
    }
}

fn discard_live_candidate_on_close(state: &mut LivePublisherState, reason: PublisherCloseReason) {
    if candidate_needs_publication(state) {
        state
            .diagnostics
            .push_back(LivePublisherDiagnostic::DraftDiscardedOnClose { reason });
    }
    state.candidate = None;
}

fn process_live_candidate(shared: &LivePublisherShared, selected: LiveCandidate) -> AppResult<()> {
    let permit = shared
        .pacer
        .wait_for_turn(Some(&shared.interrupt_text_wait))?;
    let Some(permit) = permit else {
        return Ok(());
    };

    let output_guard = shared
        .output_gate
        .lock()
        .map_err(|_| AppError::state("Live publisher output gate was poisoned."))?;
    let mut selection_result = None;
    let mut send_result: Option<AppResult<ChatboxSendReceipt>> = None;
    let committed = shared.generation.commit_if_active(|| {
        let selected_is_current = shared
            .state
            .lock()
            .map_err(|_| AppError::state("Live publisher state lock was poisoned."))
            .map(|mut state| {
                let is_current = state.lifecycle == PublisherLifecycle::Running
                    && state.candidate.as_ref().is_some_and(|candidate| {
                        candidate.identity == selected.identity && candidate.view == selected.view
                    })
                    && state
                        .last_attempted
                        .as_ref()
                        .is_none_or(|attempt| !attempt.matches(&selected))
                    && state.last_published.as_ref().is_none_or(|published| {
                        published.scope != selected.identity.scope
                            || published.view != selected.view
                    })
                    && shared.pacer.now() >= selected.ready_at;
                if is_current {
                    state.last_attempted = Some(LiveCandidateAttempt {
                        identity: selected.identity.clone(),
                        view: selected.view.clone(),
                    });
                }
                is_current
            });
        let should_send = matches!(selected_is_current, Ok(true));
        selection_result = Some(selected_is_current);
        if should_send {
            send_result = Some(permit.attempt(|| shared.transport.send_text(&selected.view)));
        }
    })?;
    if !committed {
        drop(output_guard);
        thread::yield_now();
        return Ok(());
    }

    let selected_is_current = selection_result.ok_or_else(|| {
        AppError::state("Live publisher committed without validating its candidate.")
    })??;
    if !selected_is_current {
        return Ok(());
    }

    let Some(send_result) = send_result else {
        return Err(AppError::state(
            "Live publisher committed without a transport result.",
        ));
    };
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;
    match send_result {
        Ok(receipt) => {
            state.last_published = Some(PublishedLiveView {
                scope: selected.identity.scope.clone(),
                view: selected.view,
            });
            state
                .diagnostics
                .push_back(LivePublisherDiagnostic::ViewPublished {
                    stream_id: selected.identity.scope.stream_id,
                    unit_id: selected.identity.scope.unit_id,
                    revision: selected.identity.revision,
                    byte_count: receipt.byte_count,
                    target: receipt.target,
                });
        }
        Err(error) => {
            state
                .diagnostics
                .push_back(LivePublisherDiagnostic::ViewSendFailed {
                    stream_id: selected.identity.scope.stream_id,
                    unit_id: selected.identity.scope.unit_id,
                    revision: selected.identity.revision,
                    error,
                });
        }
    }
    refresh_typing_desired(&mut state);
    shared.wake.notify_all();
    Ok(())
}

fn process_typing(shared: &LivePublisherShared, epoch: u64, is_typing: bool) -> AppResult<()> {
    let output_guard = shared
        .output_gate
        .lock()
        .map_err(|_| AppError::state("Live publisher output gate was poisoned."))?;
    let should_attempt = {
        let state = shared
            .state
            .lock()
            .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;
        state.lifecycle == PublisherLifecycle::Running
            && state.typing_epoch == epoch
            && state.typing_desired == is_typing
    };
    if !should_attempt {
        return Ok(());
    }

    let mut transport_result = None;
    let committed = shared.generation.commit_if_active(|| {
        transport_result = Some(shared.transport.send_typing(is_typing));
    })?;
    if !committed {
        drop(output_guard);
        thread::yield_now();
        return Ok(());
    }

    let Some(result) = transport_result else {
        return Err(AppError::state(
            "Live publisher committed typing without a transport result.",
        ));
    };
    let attempted_at = shared.pacer.now();
    let mut state = shared
        .state
        .lock()
        .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;
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
            .push_back(LivePublisherDiagnostic::TypingFailed { error });
    }
    shared.wake.notify_all();
    Ok(())
}

fn process_cleanup_typing(shared: &LivePublisherShared) -> AppResult<()> {
    let result = shared.transport.send_typing(false);
    if let Err(error) = result {
        let mut state = shared
            .state
            .lock()
            .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?;
        state
            .diagnostics
            .push_back(LivePublisherDiagnostic::TypingFailed { error });
        shared.wake.notify_all();
    }
    Ok(())
}

fn refresh_typing_desired(state: &mut LivePublisherState) {
    let desired = !state.unit_first_seen.is_empty() || candidate_needs_publication(state);
    if desired != state.typing_desired {
        state.typing_desired = desired;
        state.typing_epoch = state.typing_epoch.wrapping_add(1);
        state.typing_attempted_epoch = None;
        state.next_typing_reassert_at = None;
    }
}

fn candidate_needs_publication(state: &LivePublisherState) -> bool {
    state.candidate.as_ref().is_some_and(|candidate| {
        state
            .last_attempted
            .as_ref()
            .is_none_or(|attempt| !attempt.matches(candidate))
            && state.last_published.as_ref().is_none_or(|published| {
                published.scope != candidate.identity.scope || published.view != candidate.view
            })
    })
}

fn compose_recent_source(captions: &[&CaptionSnapshotV1]) -> String {
    captions
        .iter()
        .rev()
        .map(|caption| caption.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
