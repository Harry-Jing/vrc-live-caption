//! Path-independent active recognition boundary.
//!
//! Runtime submits continuous owned audio to a generation-scoped Module and
//! consumes normalized signals. Concrete drivers own speech boundaries,
//! attempts, transport or worker I/O, reconnect, and hard-stop cleanup.

mod openai;
#[cfg(test)]
mod scripted_events;

use crate::caption::{CaptionSnapshot, CaptionState};
use crate::error::{AppError, AppResult};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(crate) use openai::{openai_gpt_live_transcribe_module, openai_gpt_transcribe_module};
#[cfg(test)]
pub(crate) use scripted_events::{
    ScriptedRecognitionContext, ScriptedRecognitionEvents, ScriptedText,
};

const RECOGNITION_SIGNAL_QUEUE_CAPACITY: usize = 128;
const RECOGNITION_SIGNAL_CONTROL_RESERVE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionEvent {
    UnitStarted {
        generation: u64,
        stream_id: String,
        unit_id: String,
        started_at_ms: u64,
    },
    /// The source unit closed without an accepted Completed caption. Normal
    /// completion is represented by `CaptionState::Completed`, not this event.
    UnitAborted {
        generation: u64,
        stream_id: String,
        unit_id: String,
        reason: RecognitionUnitAbortReason,
    },
    Caption(CaptionSnapshot),
}

/// Why a confirmed source unit produced no Completed caption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionUnitAbortReason {
    NoSpeech,
    Failed { detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecognitionGenerationScope {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
}

#[derive(Debug, PartialEq)]
pub(crate) struct OwnedRecognitionAudioFrame {
    pub(crate) sequence: u64,
    pub(crate) captured_at_ms: u64,
    pub(crate) sample_rate_hz: u32,
    pub(crate) samples: Box<[f32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionSignal {
    Ready {
        generation: u64,
        stream_id: String,
        recovered: bool,
    },
    Reconnecting {
        epoch: u64,
        retry_number: u32,
        delay_ms: u64,
    },
    Event(RecognitionEvent),
}

struct RecognitionSignalQueue {
    state: Mutex<RecognitionSignalQueueState>,
    wake: Condvar,
}

struct RecognitionSignalQueueState {
    pending: VecDeque<RecognitionSignal>,
    sender_alive: bool,
    receiver_alive: bool,
}

struct RecognitionSignalSender {
    queue: Arc<RecognitionSignalQueue>,
}

enum RecognitionSignalSendError {
    Full,
    Disconnected,
}

pub(crate) struct RecognitionSignalReceiver {
    queue: Arc<RecognitionSignalQueue>,
}

fn recognition_signal_queue() -> (RecognitionSignalSender, RecognitionSignalReceiver) {
    let queue = Arc::new(RecognitionSignalQueue {
        state: Mutex::new(RecognitionSignalQueueState {
            pending: VecDeque::with_capacity(RECOGNITION_SIGNAL_QUEUE_CAPACITY),
            sender_alive: true,
            receiver_alive: true,
        }),
        wake: Condvar::new(),
    });
    (
        RecognitionSignalSender {
            queue: Arc::clone(&queue),
        },
        RecognitionSignalReceiver { queue },
    )
}

impl RecognitionSignalSender {
    fn try_send(&self, signal: RecognitionSignal) -> Result<(), RecognitionSignalSendError> {
        let mut state = match self.queue.state.lock() {
            Ok(state) => state,
            Err(_) => return Err(RecognitionSignalSendError::Disconnected),
        };
        if !state.receiver_alive {
            return Err(RecognitionSignalSendError::Disconnected);
        }

        if let Some(existing_index) = state
            .pending
            .iter()
            .position(|existing| same_ongoing_caption(existing, &signal))
        {
            let _replaced = state.pending.remove(existing_index);
            state.pending.push_back(signal);
            drop(state);
            self.queue.wake.notify_one();
            return Ok(());
        }

        let limit = if is_ongoing_caption(&signal) {
            RECOGNITION_SIGNAL_QUEUE_CAPACITY - RECOGNITION_SIGNAL_CONTROL_RESERVE
        } else {
            RECOGNITION_SIGNAL_QUEUE_CAPACITY
        };
        if state.pending.len() >= limit {
            return Err(RecognitionSignalSendError::Full);
        }
        state.pending.push_back(signal);
        drop(state);
        self.queue.wake.notify_one();
        Ok(())
    }
}

impl Drop for RecognitionSignalSender {
    fn drop(&mut self) {
        if let Ok(mut state) = self.queue.state.lock() {
            state.sender_alive = false;
        }
        self.queue.wake.notify_all();
    }
}

impl RecognitionSignalReceiver {
    pub(crate) fn try_recv(&self) -> Result<RecognitionSignal, std::sync::mpsc::TryRecvError> {
        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)?;
        if let Some(signal) = state.pending.pop_front() {
            return Ok(signal);
        }
        if state.sender_alive {
            Err(std::sync::mpsc::TryRecvError::Empty)
        } else {
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        }
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<RecognitionSignal, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| RecvTimeoutError::Disconnected)?;
        loop {
            if let Some(signal) = state.pending.pop_front() {
                return Ok(signal);
            }
            if !state.sender_alive {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            let (next_state, wait) = self
                .queue
                .wake
                .wait_timeout(state, remaining)
                .map_err(|_| RecvTimeoutError::Disconnected)?;
            state = next_state;
            if wait.timed_out() && state.pending.is_empty() {
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }
}

impl Drop for RecognitionSignalReceiver {
    fn drop(&mut self) {
        if let Ok(mut state) = self.queue.state.lock() {
            state.receiver_alive = false;
            state.pending.clear();
        }
        self.queue.wake.notify_all();
    }
}

fn same_ongoing_caption(existing: &RecognitionSignal, incoming: &RecognitionSignal) -> bool {
    let (
        RecognitionSignal::Event(RecognitionEvent::Caption(existing)),
        RecognitionSignal::Event(RecognitionEvent::Caption(incoming)),
    ) = (existing, incoming)
    else {
        return false;
    };
    existing.state == CaptionState::Ongoing
        && incoming.state == CaptionState::Ongoing
        && existing.generation == incoming.generation
        && existing.stream_id == incoming.stream_id
        && existing.unit_id == incoming.unit_id
        && existing.lane == incoming.lane
}

fn is_ongoing_caption(signal: &RecognitionSignal) -> bool {
    matches!(
        signal,
        RecognitionSignal::Event(RecognitionEvent::Caption(CaptionSnapshot {
            state: CaptionState::Ongoing,
            ..
        }))
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionSubmitError {
    Backpressure,
    InvalidAudio,
    NotReady,
    Stopped,
}

pub(crate) struct RecognitionModule {
    ingress_capacity: usize,
    max_queued_audio_micros: u64,
    driver: Box<dyn RecognitionDriver>,
}

impl RecognitionModule {
    pub(crate) fn with_audio_budget(
        max_queued_audio: Duration,
        max_queued_frames: usize,
        driver: impl RecognitionDriver,
    ) -> AppResult<Self> {
        if max_queued_audio.is_zero() {
            return Err(AppError::state(
                "Recognition audio budget must be greater than zero.",
            ));
        }
        if max_queued_frames == 0 {
            return Err(AppError::state(
                "Recognition frame capacity must be greater than zero.",
            ));
        }
        let max_queued_audio_micros =
            u64::try_from(max_queued_audio.as_micros()).map_err(|_| {
                AppError::state("Recognition audio budget exceeds the supported duration.")
            })?;

        Ok(Self {
            ingress_capacity: max_queued_frames,
            max_queued_audio_micros,
            driver: Box::new(driver),
        })
    }

    pub(crate) fn start(self, scope: RecognitionGenerationScope) -> AppResult<RunningRecognition> {
        let (ingress, input) = sync_channel(self.ingress_capacity);
        let (signal_sender, signals) = recognition_signal_queue();
        let stopped = Arc::new(RecognitionStopState::default());
        let audio_budget = Arc::new(RecognitionAudioBudget::new(self.max_queued_audio_micros));
        let driver_stopped = Arc::clone(&stopped);
        let thread_scope = scope.clone();
        let driver = self.driver;
        let worker = thread::Builder::new()
            .name(format!("vrc-live-caption-recognition-{}", scope.generation))
            .spawn(move || {
                let _admission_guard = RecognitionAdmissionGuard {
                    stopped: Arc::clone(&driver_stopped),
                };
                driver.run(RecognitionDriverIo {
                    scope: thread_scope,
                    input,
                    signals: signal_sender,
                    stopped: driver_stopped,
                })
            })
            .map_err(|error| {
                AppError::runtime(format!(
                    "Failed to start the Recognition Module owner: {error}"
                ))
            })?;

        Ok(RunningRecognition {
            ingress: Some(ingress),
            signals,
            stopped,
            audio_budget,
            worker: Some(worker),
        })
    }
}

pub(crate) struct RunningRecognition {
    ingress: Option<SyncSender<QueuedRecognitionAudioFrame>>,
    pub(crate) signals: RecognitionSignalReceiver,
    stopped: Arc<RecognitionStopState>,
    audio_budget: Arc<RecognitionAudioBudget>,
    worker: Option<JoinHandle<AppResult<()>>>,
}

#[cfg(test)]
/// Passive witness for the Module-owned admission guard's terminal state.
///
/// A Driver-local drop signal fires before `RecognitionAdmissionGuard` closes
/// admission, so it cannot establish the pre-activation ordering that Runtime
/// coordinator tests need to drive without scheduler luck.
#[derive(Clone)]
pub(crate) struct RecognitionOwnerTerminationObserver {
    stopped: Arc<RecognitionStopState>,
}

impl RunningRecognition {
    pub(crate) fn try_submit(
        &self,
        frame: OwnedRecognitionAudioFrame,
    ) -> Result<(), RecognitionSubmitError> {
        let queued = self.prepare_submission(frame)?;
        self.send_prepared_submission(queued)
    }

    #[cfg(test)]
    pub(crate) fn try_submit_with_hook(
        &self,
        frame: OwnedRecognitionAudioFrame,
        after_admission: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<Result<(), RecognitionSubmitError>> {
        let queued = match self.prepare_submission(frame) {
            Ok(queued) => queued,
            Err(error) => return Ok(Err(error)),
        };
        after_admission()?;
        Ok(self.send_prepared_submission(queued))
    }

    fn prepare_submission(
        &self,
        frame: OwnedRecognitionAudioFrame,
    ) -> Result<QueuedRecognitionAudioFrame, RecognitionSubmitError> {
        if self.stopped.is_requested() || self.stopped.is_terminated() {
            return Err(RecognitionSubmitError::Stopped);
        }
        if self.ingress.is_none() {
            return Err(RecognitionSubmitError::Stopped);
        }
        let Some(audio_micros) = frame.duration_micros() else {
            return Err(RecognitionSubmitError::InvalidAudio);
        };
        let admission_epoch = self.stopped.admission_epoch()?;
        let Some(permit) = RecognitionAudioBudget::try_reserve(&self.audio_budget, audio_micros)
        else {
            self.stopped.request();
            return Err(RecognitionSubmitError::Backpressure);
        };
        Ok(QueuedRecognitionAudioFrame {
            admission_epoch,
            frame,
            _permit: permit,
        })
    }

    fn send_prepared_submission(
        &self,
        queued: QueuedRecognitionAudioFrame,
    ) -> Result<(), RecognitionSubmitError> {
        let Some(ingress) = self.ingress.as_ref() else {
            return Err(RecognitionSubmitError::Stopped);
        };
        match ingress.try_send(queued) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.stopped.request();
                Err(RecognitionSubmitError::Backpressure)
            }
            Err(TrySendError::Disconnected(_)) => Err(RecognitionSubmitError::Stopped),
        }
    }

    pub(crate) fn stop(&mut self) -> AppResult<()> {
        self.stopped.request();
        self.ingress.take();

        let worker_result = match self.worker.take() {
            Some(worker) => match worker.join() {
                Ok(result) => result,
                Err(_) => Err(AppError::runtime(
                    "Recognition Module owner thread panicked.",
                )),
            },
            None => Ok(()),
        };
        while self.signals.try_recv().is_ok() {}
        worker_result
    }

    pub(crate) fn is_accepting_audio(&self) -> bool {
        self.stopped.is_accepting_audio()
    }

    #[cfg(test)]
    pub(crate) fn owner_termination_observer(&self) -> RecognitionOwnerTerminationObserver {
        RecognitionOwnerTerminationObserver {
            stopped: Arc::clone(&self.stopped),
        }
    }

    pub(crate) fn acknowledge_capture_paused(&self, epoch: u64) -> AppResult<()> {
        self.stopped.acknowledge_capture_paused(epoch)
    }
}

#[cfg(test)]
impl RecognitionOwnerTerminationObserver {
    pub(crate) fn wait_for_termination(&self, timeout: Duration) -> AppResult<()> {
        self.stopped.wait_for_termination(timeout)
    }
}

impl Drop for RunningRecognition {
    fn drop(&mut self) {
        self.stopped.request();
        self.ingress.take();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::warn!("Recognition Module owner thread panicked during Drop");
        }
        while self.signals.try_recv().is_ok() {}
    }
}

pub(crate) trait RecognitionDriver: Send + 'static {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()>;
}

pub(crate) struct RecognitionDriverIo {
    scope: RecognitionGenerationScope,
    input: Receiver<QueuedRecognitionAudioFrame>,
    signals: RecognitionSignalSender,
    stopped: Arc<RecognitionStopState>,
}

impl RecognitionDriverIo {
    pub(crate) fn scope(&self) -> &RecognitionGenerationScope {
        &self.scope
    }

    pub(crate) fn receive(&self, timeout: Duration) -> AppResult<RecognitionDriverInput> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.stopped.is_requested() {
                return Ok(RecognitionDriverInput::Stopped);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(RecognitionDriverInput::Idle);
            }

            match self.input.recv_timeout(remaining) {
                Ok(queued) => {
                    let QueuedRecognitionAudioFrame {
                        admission_epoch,
                        frame,
                        _permit,
                    } = queued;
                    drop(_permit);
                    if self.stopped.is_requested() {
                        return Ok(RecognitionDriverInput::Stopped);
                    }
                    if admission_epoch != self.stopped.current_admission_epoch()? {
                        continue;
                    }
                    return Ok(RecognitionDriverInput::Audio(frame));
                }
                Err(RecvTimeoutError::Timeout) => return Ok(RecognitionDriverInput::Idle),
                Err(RecvTimeoutError::Disconnected) => {
                    return Ok(RecognitionDriverInput::Stopped);
                }
            }
        }
    }

    pub(crate) fn emit(&self, signal: RecognitionSignal) -> AppResult<()> {
        if self.stopped.is_requested() {
            return Err(AppError::state(
                "Recognition Module has stopped accepting signals.",
            ));
        }

        match self.signals.try_send(signal) {
            Ok(()) => Ok(()),
            Err(RecognitionSignalSendError::Full) => Err(AppError::recognition_backpressure(
                "The bounded recognition signal queue filled.",
            )),
            Err(RecognitionSignalSendError::Disconnected) => {
                Err(AppError::state("Recognition signal receiver disconnected."))
            }
        }
    }

    pub(crate) fn ready(&self, recovered: bool) -> AppResult<()> {
        self.stopped.open_admission()?;
        let result = self.emit(RecognitionSignal::Ready {
            generation: self.scope.generation,
            stream_id: self.scope.stream_id.clone(),
            recovered,
        });
        if result.is_err() {
            self.stopped.close_admission();
        }
        result
    }

    pub(crate) fn emit_event(&self, event: RecognitionEvent) -> AppResult<()> {
        self.emit(RecognitionSignal::Event(event))
    }

    pub(crate) fn reconnecting(
        &self,
        epoch: u64,
        retry_number: u32,
        delay: Duration,
    ) -> AppResult<()> {
        self.stopped.begin_capture_pause(epoch)?;
        self.discard_pending_audio();
        let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.emit(RecognitionSignal::Reconnecting {
            epoch,
            retry_number,
            delay_ms,
        })?;
        self.stopped.wait_for_capture_pause(epoch)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.is_requested()
    }

    pub(crate) fn wait_for_stop(&self, timeout: Duration) -> AppResult<bool> {
        self.stopped.wait_timeout(timeout)
    }

    #[cfg(test)]
    pub(crate) fn wait_until_stopped(&self) -> AppResult<()> {
        self.stopped.wait()
    }

    fn discard_pending_audio(&self) {
        while self.input.try_recv().is_ok() {}
    }
}

impl OwnedRecognitionAudioFrame {
    fn duration_micros(&self) -> Option<u64> {
        if self.sample_rate_hz == 0 || self.samples.is_empty() {
            return None;
        }
        let samples = u64::try_from(self.samples.len()).ok()?;
        Some(
            samples
                .saturating_mul(1_000_000)
                .div_ceil(u64::from(self.sample_rate_hz)),
        )
    }
}

struct RecognitionAudioBudget {
    max_micros: u64,
    queued_micros: AtomicU64,
}

impl RecognitionAudioBudget {
    fn new(max_micros: u64) -> Self {
        Self {
            max_micros,
            queued_micros: AtomicU64::new(0),
        }
    }

    fn try_reserve(budget: &Arc<Self>, audio_micros: u64) -> Option<RecognitionAudioPermit> {
        budget
            .queued_micros
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |queued| {
                let updated = queued.checked_add(audio_micros)?;
                (updated <= budget.max_micros).then_some(updated)
            })
            .ok()
            .map(|_| RecognitionAudioPermit {
                budget: Arc::clone(budget),
                audio_micros,
            })
    }

    fn release(&self, audio_micros: u64) {
        let previous = self.queued_micros.fetch_sub(audio_micros, Ordering::SeqCst);
        debug_assert!(previous >= audio_micros);
    }
}

struct RecognitionAudioPermit {
    budget: Arc<RecognitionAudioBudget>,
    audio_micros: u64,
}

impl Drop for RecognitionAudioPermit {
    fn drop(&mut self) {
        self.budget.release(self.audio_micros);
    }
}

struct QueuedRecognitionAudioFrame {
    admission_epoch: u64,
    frame: OwnedRecognitionAudioFrame,
    _permit: RecognitionAudioPermit,
}

pub(crate) enum RecognitionDriverInput {
    Audio(OwnedRecognitionAudioFrame),
    Idle,
    Stopped,
}

#[derive(Default)]
struct RecognitionStopState {
    requested: AtomicBool,
    terminated: AtomicBool,
    wait_lock: Mutex<RecognitionWaitState>,
    wake: Condvar,
}

#[derive(Default)]
struct RecognitionWaitState {
    admission_epoch: u64,
    accepting_audio: bool,
    pending_pause_epoch: Option<u64>,
}

struct RecognitionAdmissionGuard {
    stopped: Arc<RecognitionStopState>,
}

impl Drop for RecognitionAdmissionGuard {
    fn drop(&mut self) {
        self.stopped.mark_terminated();
    }
}

impl RecognitionStopState {
    fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    fn is_terminated(&self) -> bool {
        self.terminated.load(Ordering::SeqCst)
    }

    fn mark_terminated(&self) {
        self.terminated.store(true, Ordering::SeqCst);
        self.close_admission();
        self.wake.notify_all();
    }

    fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.close_admission();
        self.wake.notify_all();
    }

    fn is_accepting_audio(&self) -> bool {
        if self.is_requested() || self.is_terminated() {
            return false;
        }
        self.wait_lock
            .lock()
            .map(|state| state.accepting_audio)
            .unwrap_or(false)
    }

    fn admission_epoch(&self) -> Result<u64, RecognitionSubmitError> {
        if self.is_requested() || self.is_terminated() {
            return Err(RecognitionSubmitError::Stopped);
        }
        let state = self
            .wait_lock
            .lock()
            .map_err(|_| RecognitionSubmitError::Stopped)?;
        if self.is_requested() || self.is_terminated() {
            return Err(RecognitionSubmitError::Stopped);
        }
        if !state.accepting_audio {
            return Err(RecognitionSubmitError::NotReady);
        }
        Ok(state.admission_epoch)
    }

    fn current_admission_epoch(&self) -> AppResult<u64> {
        self.wait_lock
            .lock()
            .map(|state| state.admission_epoch)
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))
    }

    fn open_admission(&self) -> AppResult<()> {
        let mut state = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        if self.is_requested() {
            return Err(AppError::state(
                "Stopped recognition cannot reopen audio admission.",
            ));
        }
        if state.pending_pause_epoch.is_some() {
            return Err(AppError::state(
                "Recognition cannot reopen audio before capture pause is acknowledged.",
            ));
        }
        state.accepting_audio = true;
        Ok(())
    }

    fn close_admission(&self) {
        if let Ok(mut state) = self.wait_lock.lock() {
            state.accepting_audio = false;
        }
    }

    fn begin_capture_pause(&self, epoch: u64) -> AppResult<()> {
        let mut state = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        if state.pending_pause_epoch.is_some() {
            return Err(AppError::state(
                "Recognition already has a capture pause awaiting acknowledgement.",
            ));
        }
        state.accepting_audio = false;
        state.admission_epoch = state.admission_epoch.saturating_add(1);
        state.pending_pause_epoch = Some(epoch);
        Ok(())
    }

    fn acknowledge_capture_paused(&self, epoch: u64) -> AppResult<()> {
        let mut state = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        if state.pending_pause_epoch != Some(epoch) {
            return Err(AppError::state(
                "Capture pause acknowledgement did not match the pending reconnect epoch.",
            ));
        }
        state.pending_pause_epoch = None;
        self.wake.notify_all();
        Ok(())
    }

    fn wait_for_capture_pause(&self, epoch: u64) -> AppResult<()> {
        let mut state = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        while state.pending_pause_epoch == Some(epoch) && !self.is_requested() {
            state = self
                .wake
                .wait(state)
                .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn wait(&self) -> AppResult<()> {
        let mut guard = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        while !self.is_requested() {
            guard = self
                .wake
                .wait(guard)
                .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn wait_for_termination(&self, timeout: Duration) -> AppResult<()> {
        let guard = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        if self.is_terminated() {
            return Ok(());
        }
        // `mark_terminated` closes admission by acquiring this same wait lock
        // before notifying. The predicate is rechecked while the lock is held,
        // so termination cannot land between the check and sleep as a lost wake.
        let (_guard, wait_result) = self
            .wake
            .wait_timeout_while(guard, timeout, |_| !self.is_terminated())
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        if wait_result.timed_out() && !self.is_terminated() {
            return Err(AppError::state(
                "Recognition owner did not terminate before the test watchdog expired.",
            ));
        }
        Ok(())
    }

    fn wait_timeout(&self, timeout: Duration) -> AppResult<bool> {
        let guard = self
            .wait_lock
            .lock()
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        if self.is_requested() {
            return Ok(true);
        }
        let (_guard, _timeout) = self
            .wake
            .wait_timeout_while(guard, timeout, |_| !self.is_requested())
            .map_err(|_| AppError::state("Recognition lifecycle wait lock was poisoned."))?;
        Ok(self.is_requested())
    }
}

#[cfg(test)]
#[path = "recognition/recognition_tests.rs"]
mod tests;
