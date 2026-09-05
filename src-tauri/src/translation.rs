//! Bounded provider-neutral completed-text translation owner.
//!
//! Runtime admits exact completed-Source reservations without waiting for the
//! provider. This owner keeps correlation, resource limits, retries, and Stop
//! semantics on the application side of the provider boundary.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "GitHub issue #25 activates prepared Translation at desktop Start."
    )
)]

use crate::caption::{CaptionAggregateUpdate, ReservedCompletedSource, TranslationFailureReason};
use crate::config::{TranslationConfig, TranslationPath, TranslationTarget};
use crate::credentials::{CredentialId, CredentialStorage, ResolvedCredential};
use crate::error::{AppError, AppResult};
use crate::host_resolver::HostResolver;
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod openai_responses;

const OUTSTANDING_LIMIT: usize = 8;
const RETAINED_SOURCE_BYTE_LIMIT: usize = 64 * 1024;
const SOURCE_BYTE_LIMIT: usize = 16 * 1024;
const TRANSLATION_BYTE_LIMIT: usize = 32 * 1024;
const TOTAL_DEADLINE: Duration = Duration::from_secs(12);
const ATTEMPT_DEADLINE: Duration = Duration::from_secs(5);
const MAX_ATTEMPTS: u8 = 2;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TranslationSourceRef {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) unit_id: String,
    pub(crate) revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TranslationFailureClass {
    Authentication,
    PermissionDenied,
    InvalidRequest,
    RateLimited,
    UsageLimit,
    ServiceUnavailable,
    InvalidOutput,
    DeadlineExceeded,
    Unknown,
}

impl TranslationFailureClass {
    const fn reason(self) -> TranslationFailureReason {
        match self {
            Self::Authentication => TranslationFailureReason::ProviderAuthenticationFailed,
            Self::PermissionDenied => TranslationFailureReason::ProviderPermissionDenied,
            Self::InvalidRequest => TranslationFailureReason::ProviderInvalidRequest,
            Self::RateLimited => TranslationFailureReason::ProviderRateLimited,
            Self::UsageLimit => TranslationFailureReason::ProviderUsageLimit,
            Self::ServiceUnavailable => TranslationFailureReason::ProviderUnavailable,
            Self::InvalidOutput => TranslationFailureReason::InvalidOutput,
            Self::DeadlineExceeded => TranslationFailureReason::DeadlineExceeded,
            Self::Unknown => TranslationFailureReason::Failed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TranslationSubmitError {
    OutstandingLimit,
    RetainedSourceLimit,
    SourceTooLarge,
    InvalidSource,
    Closed,
    Stopped,
}

impl TranslationSubmitError {
    const fn reason(self) -> TranslationFailureReason {
        match self {
            Self::OutstandingLimit | Self::RetainedSourceLimit => {
                TranslationFailureReason::Backpressure
            }
            Self::SourceTooLarge => TranslationFailureReason::SourceTooLarge,
            Self::Stopped => TranslationFailureReason::Stopped,
            Self::InvalidSource | Self::Closed => TranslationFailureReason::Failed,
        }
    }
}

/// A rejected admission still owns the exact Aggregate terminalization right.
/// `fail` records why before releasing the reservation; dropping the fallback
/// capability records a stable stopped outcome rather than erasing pending.
pub(crate) struct TranslationSubmissionRejection {
    kind: TranslationSubmitError,
    reservation: Box<ReservedCompletedSource>,
}

impl TranslationSubmissionRejection {
    fn new(kind: TranslationSubmitError, reservation: ReservedCompletedSource) -> Self {
        Self {
            kind,
            reservation: Box::new(reservation),
        }
    }

    pub(crate) const fn kind(&self) -> TranslationSubmitError {
        self.kind
    }

    pub(crate) const fn reason(&self) -> TranslationFailureReason {
        self.kind.reason()
    }

    pub(crate) fn fail(self) -> AppResult<Option<CaptionAggregateUpdate>> {
        let reason = self.reason();
        (*self.reservation).fail_translation(reason)
    }
}

impl fmt::Debug for TranslationSubmissionRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranslationSubmissionRejection")
            .field("kind", &self.kind)
            .field("reservation", &self.reservation)
            .finish()
    }
}

pub(crate) enum TranslationTerminalOutcome {
    Completed(Box<CompletedTranslation>),
    Failed(FailedTranslation),
}

impl TranslationTerminalOutcome {
    pub(crate) fn source_ref(&self) -> &TranslationSourceRef {
        match self {
            Self::Completed(completed) => &completed.source_ref,
            Self::Failed(failed) => &failed.source_ref,
        }
    }
}

impl fmt::Debug for TranslationTerminalOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed(completed) => {
                formatter.debug_tuple("Completed").field(completed).finish()
            }
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

pub(crate) struct CompletedTranslation {
    source_ref: TranslationSourceRef,
    reservation: ReservedCompletedSource,
    text: String,
    target: TranslationTarget,
    // A successful outcome remains admitted after the receiver pops it. The
    // slot and retained Source bytes are released only when the consumer
    // finalizes or drops this value.
    _permit: TranslationPermit,
}

impl CompletedTranslation {
    pub(crate) fn complete(self, timestamp_ms: u64) -> AppResult<Option<CaptionAggregateUpdate>> {
        self.reservation.complete_translation(
            self.text,
            translation_language_tag(self.target).to_string(),
            timestamp_ms,
        )
    }
}

impl fmt::Debug for CompletedTranslation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletedTranslation")
            .field("source_ref", &self.source_ref)
            .field("translation_bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct FailedTranslation {
    pub(crate) source_ref: TranslationSourceRef,
    pub(crate) class: TranslationFailureClass,
    reservation: Box<ReservedCompletedSource>,
    // Failure keeps both budgets until the consumer atomically records the
    // terminal Aggregate state or drops this one-shot capability.
    _permit: TranslationPermit,
}

impl FailedTranslation {
    pub(crate) const fn reason(&self) -> TranslationFailureReason {
        self.class.reason()
    }

    pub(crate) fn fail(self) -> AppResult<Option<CaptionAggregateUpdate>> {
        let reason = self.reason();
        (*self.reservation).fail_translation(reason)
    }
}

impl fmt::Debug for FailedTranslation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FailedTranslation")
            .field("source_ref", &self.source_ref)
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}

pub(crate) struct TranslationOutcomeReceiver {
    shared: Arc<TranslationShared>,
}

impl TranslationOutcomeReceiver {
    pub(crate) fn try_recv(
        &self,
    ) -> Result<TranslationTerminalOutcome, std::sync::mpsc::TryRecvError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)?;
        if let Some(envelope) = state.outcomes.pop_front() {
            return Ok(envelope.outcome);
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
    ) -> Result<TranslationTerminalOutcome, RecvTimeoutError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| RecvTimeoutError::Disconnected)?;
        loop {
            if let Some(envelope) = state.outcomes.pop_front() {
                return Ok(envelope.outcome);
            }
            if !state.sender_alive {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            let (next_state, wait) = self
                .shared
                .wake
                .wait_timeout(state, remaining)
                .map_err(|_| RecvTimeoutError::Disconnected)?;
            state = next_state;
            if wait.timed_out() && state.outcomes.is_empty() {
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }
}

impl Drop for TranslationOutcomeReceiver {
    fn drop(&mut self) {
        self.shared.request_stop();
    }
}

pub(crate) struct TranslationModule {
    shared: Arc<TranslationShared>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(test)]
mod owner_quiescence {
    use super::*;

    /// Opaque registration for the exact owner that may call a test adapter.
    pub(super) struct TranslationOwnerRegistration(std::sync::Weak<TranslationShared>);

    /// Opaque proof that the exact registered owner has completed Stop/join.
    pub(super) struct TranslationOwnerQuiesced(Arc<TranslationShared>);

    impl TranslationOwnerRegistration {
        pub(super) fn matches_module(&self, module: &TranslationModule) -> bool {
            std::sync::Weak::ptr_eq(&self.0, &Arc::downgrade(&module.shared))
        }

        pub(super) fn accepts(&self, proof: &TranslationOwnerQuiesced) -> bool {
            self.0
                .upgrade()
                .is_some_and(|shared| Arc::ptr_eq(&shared, &proof.0))
        }
    }

    impl TranslationModule {
        pub(super) fn owner_registration(&self) -> TranslationOwnerRegistration {
            TranslationOwnerRegistration(Arc::downgrade(&self.shared))
        }

        pub(super) fn stop_and_confirm_owner_quiesced(
            &mut self,
        ) -> AppResult<TranslationOwnerQuiesced> {
            self.stop()?;
            Ok(TranslationOwnerQuiesced(Arc::clone(&self.shared)))
        }
    }
}

#[cfg(test)]
use owner_quiescence::{TranslationOwnerQuiesced, TranslationOwnerRegistration};

/// Provider owner and metadata created from one credential-bound selection.
///
/// Its private fields prevent Runtime from relabeling a Module after the
/// provider target, endpoint, and secret have already been captured.
pub(crate) struct BoundTranslationModule {
    selection: TranslationConfig,
    credential_id: CredentialId,
    credential_storage: CredentialStorage,
    credential_display_suffix: Option<String>,
    credential_revision: u64,
    module: TranslationModule,
    outcomes: TranslationOutcomeReceiver,
}

pub(crate) struct BoundTranslationParts {
    pub(crate) selection: TranslationConfig,
    pub(crate) credential_id: CredentialId,
    pub(crate) credential_storage: CredentialStorage,
    pub(crate) credential_display_suffix: Option<String>,
    pub(crate) credential_revision: u64,
    pub(crate) module: TranslationModule,
    pub(crate) outcomes: TranslationOutcomeReceiver,
}

impl BoundTranslationModule {
    pub(crate) fn into_parts(self) -> BoundTranslationParts {
        BoundTranslationParts {
            selection: self.selection,
            credential_id: self.credential_id,
            credential_storage: self.credential_storage,
            credential_display_suffix: self.credential_display_suffix,
            credential_revision: self.credential_revision,
            module: self.module,
            outcomes: self.outcomes,
        }
    }

    #[cfg(test)]
    fn stop_for_test(mut self) -> AppResult<()> {
        self.module.stop()
    }
}

/// Prepares the one Phase 5 completed-text path without activating Runtime.
///
/// The credential remains bound to the endpoint that selected it, while the
/// provider and transport stay private behind the provider-neutral owner.
pub(crate) fn openai_responses_completed_text_module(
    selection: TranslationConfig,
    credential: ResolvedCredential,
    credential_revision: u64,
    resolver: HostResolver,
) -> AppResult<BoundTranslationModule> {
    match selection.path {
        TranslationPath::OpenAiResponsesCompletedText => {}
    }
    let credential_id = credential.id;
    let credential_storage = credential.storage;
    let credential_display_suffix = credential.display_suffix.clone();
    let adapter =
        openai_responses::OpenAiResponsesAdapter::new(&selection.endpoint, credential, resolver)
            .map_err(|_| AppError::runtime("Failed to prepare the Translation HTTP adapter."))?;
    let (module, outcomes) = TranslationModule::start(
        selection.target,
        Arc::new(adapter),
        PolicyDependencies::real(),
    )?;
    Ok(BoundTranslationModule {
        selection,
        credential_id,
        credential_storage,
        credential_display_suffix,
        credential_revision,
        module,
        outcomes,
    })
}

#[cfg(test)]
#[path = "translation/test_support.rs"]
mod test_support;

#[cfg(test)]
pub(crate) use test_support::{
    TestTranslationControl, TestTranslationResult, translation_module_for_test,
};

impl TranslationModule {
    fn start(
        target: TranslationTarget,
        adapter: Arc<dyn CompletedTextAdapter>,
        dependencies: PolicyDependencies,
    ) -> AppResult<(Self, TranslationOutcomeReceiver)> {
        let shared = Arc::new(TranslationShared {
            state: Mutex::new(TranslationState {
                accepting: true,
                sender_alive: true,
                next_job_id: 0,
                pending: VecDeque::with_capacity(OUTSTANDING_LIMIT),
                active: None,
                outcomes: VecDeque::with_capacity(OUTSTANDING_LIMIT),
            }),
            wake: Condvar::new(),
            attempt_start_gate: Mutex::new(()),
            stopped: AtomicBool::new(false),
            clock: Arc::clone(&dependencies.clock),
            delay: Arc::clone(&dependencies.delay),
            budget: Arc::new(TranslationBudget::default()),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("vrc-live-caption-translation".to_string())
            .spawn(move || {
                run_worker(&worker_shared, target, adapter, dependencies);
                worker_shared.mark_sender_closed();
            })
            .map_err(|error| {
                AppError::runtime(format!(
                    "Failed to start the Translation Module owner: {error}"
                ))
            })?;

        Ok((
            Self {
                shared: Arc::clone(&shared),
                worker: Some(worker),
            },
            TranslationOutcomeReceiver { shared },
        ))
    }

    #[cfg(test)]
    fn start_for_test(
        target: TranslationTarget,
        adapter: Arc<dyn CompletedTextAdapter>,
        dependencies: TestPolicyDependencies,
    ) -> AppResult<(Self, TranslationOutcomeReceiver)> {
        Self::start(target, adapter, dependencies)
    }

    pub(crate) fn try_submit(
        &self,
        reservation: ReservedCompletedSource,
    ) -> Result<(), TranslationSubmissionRejection> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::Stopped,
                reservation,
            ));
        }
        let source_bytes = reservation.source().text.len();
        if source_bytes > SOURCE_BYTE_LIMIT {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::SourceTooLarge,
                reservation,
            ));
        }
        let Some(source_ref) = source_ref(&reservation) else {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::InvalidSource,
                reservation,
            ));
        };
        let admitted_at = InstantPoint(self.shared_clock_now());
        self.try_submit_prepared(reservation, source_ref, admitted_at, source_bytes)
    }

    fn try_submit_prepared(
        &self,
        reservation: ReservedCompletedSource,
        source_ref: TranslationSourceRef,
        admitted_at: InstantPoint,
        source_bytes: usize,
    ) -> Result<(), TranslationSubmissionRejection> {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(_) => {
                return Err(TranslationSubmissionRejection::new(
                    TranslationSubmitError::Stopped,
                    reservation,
                ));
            }
        };
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::Stopped,
                reservation,
            ));
        }
        if !state.accepting {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::Closed,
                reservation,
            ));
        }
        let now = self.shared.clock.now();
        if now.saturating_sub(admitted_at.0) >= TOTAL_DEADLINE {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::Closed,
                reservation,
            ));
        }
        let permit = match TranslationBudget::try_acquire(&self.shared.budget, source_bytes) {
            Ok(permit) => permit,
            Err(limit) => {
                let kind = match limit {
                    TranslationBudgetLimit::Outstanding => TranslationSubmitError::OutstandingLimit,
                    TranslationBudgetLimit::RetainedSource => {
                        TranslationSubmitError::RetainedSourceLimit
                    }
                };
                return Err(TranslationSubmissionRejection::new(kind, reservation));
            }
        };
        state.next_job_id = state.next_job_id.saturating_add(1);
        let job_id = state.next_job_id;
        state.pending.push_back(TranslationJob {
            job_id,
            admitted_at,
            source_ref,
            reservation: Some(reservation),
            permit,
        });
        drop(state);
        self.shared.wake.notify_one();
        Ok(())
    }

    #[cfg(test)]
    fn try_submit_with_hook(
        &self,
        reservation: ReservedCompletedSource,
        after_prepare: impl FnOnce(),
    ) -> Result<(), TranslationSubmissionRejection> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::Stopped,
                reservation,
            ));
        }
        let source_bytes = reservation.source().text.len();
        if source_bytes > SOURCE_BYTE_LIMIT {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::SourceTooLarge,
                reservation,
            ));
        }
        let Some(source_ref) = source_ref(&reservation) else {
            return Err(TranslationSubmissionRejection::new(
                TranslationSubmitError::InvalidSource,
                reservation,
            ));
        };
        let admitted_at = InstantPoint(self.shared_clock_now());
        after_prepare();
        self.try_submit_prepared(reservation, source_ref, admitted_at, source_bytes)
    }

    fn shared_clock_now(&self) -> Duration {
        self.shared.clock.now()
    }

    pub(crate) fn stop(&mut self) -> AppResult<()> {
        self.shared.request_stop();
        match self.worker.take() {
            Some(worker) => worker.join().map_err(|_| {
                AppError::runtime("Translation Module owner thread panicked during Stop.")
            }),
            None => Ok(()),
        }
    }
}

impl Drop for TranslationModule {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::warn!(error = %error, "Translation Module failed while stopping during Drop");
        }
    }
}

struct TranslationShared {
    state: Mutex<TranslationState>,
    wake: Condvar,
    // Attempt authorization and Stop share this fence. An attempt is either
    // authorized before Stop linearizes or prevented from entering Adapter
    // code; a delayed attempt thread cannot independently start new work.
    attempt_start_gate: Mutex<()>,
    stopped: AtomicBool,
    clock: Arc<dyn TranslationClock>,
    delay: Arc<dyn CancellableDelay>,
    budget: Arc<TranslationBudget>,
}

struct TranslationState {
    accepting: bool,
    sender_alive: bool,
    next_job_id: u64,
    pending: VecDeque<TranslationJob>,
    active: Option<TranslationJob>,
    outcomes: VecDeque<TranslationOutcomeEnvelope>,
}

struct TranslationJob {
    job_id: u64,
    admitted_at: InstantPoint,
    source_ref: TranslationSourceRef,
    reservation: Option<ReservedCompletedSource>,
    permit: TranslationPermit,
}

struct TranslationOutcomeEnvelope {
    outcome: TranslationTerminalOutcome,
}

#[derive(Default)]
struct TranslationBudget {
    state: Mutex<TranslationBudgetState>,
}

#[derive(Default)]
struct TranslationBudgetState {
    slots: usize,
    source_bytes: usize,
}

enum TranslationBudgetLimit {
    Outstanding,
    RetainedSource,
}

impl TranslationBudget {
    fn try_acquire(
        budget: &Arc<Self>,
        source_bytes: usize,
    ) -> Result<TranslationPermit, TranslationBudgetLimit> {
        let mut state = budget
            .state
            .lock()
            .map_err(|_| TranslationBudgetLimit::Outstanding)?;
        if state.slots >= OUTSTANDING_LIMIT {
            return Err(TranslationBudgetLimit::Outstanding);
        }
        if state.source_bytes.saturating_add(source_bytes) > RETAINED_SOURCE_BYTE_LIMIT {
            return Err(TranslationBudgetLimit::RetainedSource);
        }
        state.slots += 1;
        state.source_bytes += source_bytes;
        Ok(TranslationPermit {
            budget: Arc::clone(budget),
            source_bytes,
            active: true,
        })
    }
}

struct TranslationPermit {
    budget: Arc<TranslationBudget>,
    source_bytes: usize,
    active: bool,
}

impl Drop for TranslationPermit {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Ok(mut state) = self.budget.state.lock() {
            state.slots = state.slots.saturating_sub(1);
            state.source_bytes = state.source_bytes.saturating_sub(self.source_bytes);
            self.active = false;
        }
    }
}

impl TranslationShared {
    fn request_stop(&self) {
        let _attempt_start = self.attempt_start_gate.lock().ok();
        if let Ok(mut state) = self.state.lock() {
            // Admission and Stop linearize under the same state lock. A submit
            // that returns Ok is therefore wholly before Stop, or Stop wins
            // and the submit observes the closed state.
            self.stopped.store(true, Ordering::SeqCst);
            state.accepting = false;
            state.sender_alive = false;
            state.pending.clear();
            state.active.take();
            state.outcomes.clear();
        } else {
            self.stopped.store(true, Ordering::SeqCst);
        }
        self.delay.cancel();
        self.wake.notify_all();
    }

    fn authorize_attempt_start(
        &self,
        sender: &std::sync::mpsc::SyncSender<AttemptCommand>,
    ) -> bool {
        let Ok(_attempt_start) = self.attempt_start_gate.lock() else {
            return false;
        };
        !self.stopped.load(Ordering::SeqCst) && sender.try_send(AttemptCommand::Begin).is_ok()
    }

    fn mark_sender_closed(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.accepting = false;
            state.sender_alive = false;
        }
        self.wake.notify_all();
    }
}

#[derive(Clone, Copy)]
struct InstantPoint(Duration);

trait TranslationClock: Send + Sync {
    fn now(&self) -> Duration;
}

struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl TranslationClock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

trait CancellableDelay: Send + Sync {
    fn wait(&self, duration: Duration, stopped: &AtomicBool, clock: &dyn TranslationClock) -> bool;

    fn cancel(&self) {}
}

struct SystemDelay;

impl CancellableDelay for SystemDelay {
    fn wait(
        &self,
        duration: Duration,
        stopped: &AtomicBool,
        _clock: &dyn TranslationClock,
    ) -> bool {
        let deadline = Instant::now()
            .checked_add(duration)
            .unwrap_or_else(Instant::now);
        loop {
            if stopped.load(Ordering::SeqCst) {
                return false;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return true;
            }
            thread::sleep(remaining.min(WORKER_POLL_INTERVAL));
        }
    }
}

trait RetryJitter: Send + Sync {
    fn delay(&self, base: Duration) -> Duration;
}

struct BoundedRetryJitter {
    sequence: AtomicU64,
}

impl BoundedRetryJitter {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0x9e37_79b9_7f4a_7c15),
        }
    }
}

impl RetryJitter for BoundedRetryJitter {
    fn delay(&self, base: Duration) -> Duration {
        let next = self
            .sequence
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(
                    value
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1),
                )
            })
            .unwrap_or(0);
        let base_ms = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);
        let lower = base_ms.saturating_mul(80) / 100;
        let upper = base_ms.saturating_mul(120) / 100;
        let width = upper.saturating_sub(lower).saturating_add(1);
        Duration::from_millis(lower.saturating_add(next % width))
    }
}

struct PolicyDependencies {
    clock: Arc<dyn TranslationClock>,
    delay: Arc<dyn CancellableDelay>,
    jitter: Arc<dyn RetryJitter>,
}

impl PolicyDependencies {
    fn real() -> Self {
        Self {
            clock: Arc::new(SystemClock::new()),
            delay: Arc::new(SystemDelay),
            jitter: Arc::new(BoundedRetryJitter::new()),
        }
    }
}

#[cfg(test)]
type TestPolicyDependencies = PolicyDependencies;

struct CompletedTextRequest {
    source_text: String,
    target: TranslationTarget,
}

struct AttemptControl {
    attempt_budget: Duration,
    total_budget: Duration,
}

struct AdapterCompletion {
    sender: std::sync::mpsc::SyncSender<Result<String, AdapterFailure>>,
}

impl AdapterCompletion {
    fn finish(self, result: Result<String, AdapterFailure>) {
        let _ignored = self.sender.send(result);
    }
}

struct AdapterFailure {
    class: TranslationFailureClass,
    retryable: bool,
    retry_after: Option<Duration>,
    // The adapter cannot prove whether the provider accepted the request.
    // The owner must close admission, not merely finish this one unit.
    request_outcome_ambiguous: bool,
}

trait CompletedTextAdapter: Send + Sync + 'static {
    /// Authorizes one external call and returns its cancellation handle without
    /// waiting for the provider result.
    ///
    /// Implementations must keep both `begin` and `ActiveTranslationCall::cancel`
    /// bounded and cancellation-aware. They execute on an isolated attempt
    /// thread and never own a reservation or resource permit. A completion
    /// racing with Stop is suppressed by the owner.
    fn begin(
        &self,
        request: CompletedTextRequest,
        control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure>;
}

trait ActiveTranslationCall: Send {
    /// Returns `Confirmed` only after the provider request is fully quiescent
    /// and can no longer complete. An Adapter that cannot prove that boundary
    /// must return `Unconfirmed`; the owner then closes admission instead of
    /// starting another call.
    fn cancel(&mut self) -> CancellationStatus;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancellationStatus {
    Confirmed,
    Unconfirmed,
}

fn run_worker(
    shared: &Arc<TranslationShared>,
    target: TranslationTarget,
    adapter: Arc<dyn CompletedTextAdapter>,
    dependencies: PolicyDependencies,
) {
    loop {
        let Some(work) = take_next_work(shared, dependencies.clock.as_ref()) else {
            return;
        };
        process_work(shared, work, target, Arc::clone(&adapter), &dependencies);
    }
}

struct WorkSnapshot {
    job_id: u64,
    admitted_at: InstantPoint,
    source_text: String,
}

fn take_next_work(
    shared: &Arc<TranslationShared>,
    clock: &dyn TranslationClock,
) -> Option<WorkSnapshot> {
    let mut state = shared.state.lock().ok()?;
    loop {
        if shared.stopped.load(Ordering::SeqCst) || !state.accepting {
            return None;
        }
        if let Some(mut job) = state.pending.pop_front() {
            // Admission uses a conservative timestamp before the shared worker
            // can observe the item. Replace it with no later than the real
            // policy clock's current point.
            job.admitted_at.0 = job.admitted_at.0.min(clock.now());
            let source_text = job.reservation.as_ref()?.source().text.clone();
            let work = WorkSnapshot {
                job_id: job.job_id,
                admitted_at: job.admitted_at,
                source_text,
            };
            state.active = Some(job);
            return Some(work);
        }
        state = shared.wake.wait(state).ok()?;
    }
}

fn process_work(
    shared: &Arc<TranslationShared>,
    work: WorkSnapshot,
    target: TranslationTarget,
    adapter: Arc<dyn CompletedTextAdapter>,
    dependencies: &PolicyDependencies,
) {
    let total_deadline = saturating_add(work.admitted_at.0, TOTAL_DEADLINE);
    let mut attempt = 0_u8;
    loop {
        if shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        let now = dependencies.clock.now();
        if now >= total_deadline {
            finish_failure(
                shared,
                work.job_id,
                TranslationFailureClass::DeadlineExceeded,
            );
            return;
        }
        attempt = attempt.saturating_add(1);
        let attempt_deadline = saturating_add(now, ATTEMPT_DEADLINE).min(total_deadline);
        let request = CompletedTextRequest {
            source_text: work.source_text.clone(),
            target,
        };
        let control = AttemptControl {
            attempt_budget: attempt_deadline.saturating_sub(now),
            total_budget: total_deadline.saturating_sub(now),
        };
        let attempt_result = run_attempt(
            shared,
            Arc::clone(&adapter),
            request,
            control,
            dependencies.clock.as_ref(),
            attempt_deadline,
            total_deadline,
        );
        let now = dependencies.clock.now();
        let result = match attempt_result {
            AttemptResult::Stopped => return,
            // Cancellation may be non-cooperative. Ending this job rather than
            // retrying prevents two physical provider calls from overlapping.
            AttemptResult::UnconfirmedTimeout => {
                fail_closed_after_unconfirmed_timeout(shared);
                return;
            }
            AttemptResult::Cancelled => Err(AdapterFailure {
                class: TranslationFailureClass::DeadlineExceeded,
                retryable: true,
                retry_after: None,
                request_outcome_ambiguous: false,
            }),
            // Preserve an ambiguous provider outcome even if it arrives just
            // after the owner's deadline. Reclassifying it as a normal timeout
            // would authorize an overlapping physical request.
            AttemptResult::Returned(Err(failure)) if failure.request_outcome_ambiguous => {
                Err(failure)
            }
            AttemptResult::Returned(result) if now <= attempt_deadline => result,
            AttemptResult::Returned(_) => Err(AdapterFailure {
                class: TranslationFailureClass::DeadlineExceeded,
                retryable: true,
                retry_after: None,
                request_outcome_ambiguous: false,
            }),
        };

        match result {
            Ok(text) => {
                if text.trim().is_empty() || text.len() > TRANSLATION_BYTE_LIMIT {
                    finish_failure(shared, work.job_id, TranslationFailureClass::InvalidOutput);
                } else if dependencies.clock.now() >= total_deadline {
                    finish_failure(
                        shared,
                        work.job_id,
                        TranslationFailureClass::DeadlineExceeded,
                    );
                } else {
                    finish_success(shared, work.job_id, text, target);
                }
                return;
            }
            Err(failure) => {
                if failure.request_outcome_ambiguous {
                    fail_closed_after_unconfirmed_timeout(shared);
                    return;
                }
                if !failure.retryable || attempt >= MAX_ATTEMPTS {
                    finish_failure(shared, work.job_id, failure.class);
                    return;
                }
                let now = dependencies.clock.now();
                if now >= total_deadline {
                    finish_failure(
                        shared,
                        work.job_id,
                        TranslationFailureClass::DeadlineExceeded,
                    );
                    return;
                }
                let remaining = total_deadline.saturating_sub(now);
                let requested_delay = match failure.retry_after {
                    Some(delay) if delay >= remaining => {
                        finish_failure(shared, work.job_id, failure.class);
                        return;
                    }
                    Some(delay) => delay,
                    None => dependencies.jitter.delay(RETRY_BASE_DELAY),
                };
                let delay = requested_delay.min(remaining);
                if !dependencies
                    .delay
                    .wait(delay, &shared.stopped, dependencies.clock.as_ref())
                {
                    return;
                }
                if delay == remaining {
                    finish_failure(
                        shared,
                        work.job_id,
                        TranslationFailureClass::DeadlineExceeded,
                    );
                    return;
                }
            }
        }
    }
}

enum AttemptResult {
    Returned(Result<String, AdapterFailure>),
    Cancelled,
    UnconfirmedTimeout,
    Stopped,
}

enum AttemptCommand {
    Begin,
    Cancel,
}

enum AttemptEvent {
    Completed(Result<String, AdapterFailure>),
    Cancelled(CancellationStatus),
}

fn run_attempt(
    shared: &TranslationShared,
    adapter: Arc<dyn CompletedTextAdapter>,
    request: CompletedTextRequest,
    control: AttemptControl,
    clock: &dyn TranslationClock,
    deadline: Duration,
    total_deadline: Duration,
) -> AttemptResult {
    let (event_sender, event_receiver) = sync_channel(2);
    let completion_event_sender = event_sender.clone();
    // Capacity two permits an authorized Begin and a racing Stop cancellation
    // to coexist until the attempt thread observes both in order.
    let (command_sender, command_receiver) = sync_channel(2);
    let attempt_thread = thread::Builder::new()
        .name("vrc-live-caption-translation-attempt".to_string())
        .spawn(move || {
            match command_receiver.recv() {
                Ok(AttemptCommand::Begin) => {}
                Ok(AttemptCommand::Cancel) | Err(_) => return,
            }
            match command_receiver.try_recv() {
                Ok(AttemptCommand::Cancel) => {
                    let _ignored = completion_event_sender
                        .send(AttemptEvent::Cancelled(CancellationStatus::Confirmed));
                    return;
                }
                Ok(AttemptCommand::Begin) | Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
            let (completion_sender, completion_receiver) = sync_channel(1);
            let result = adapter.begin(
                request,
                control,
                AdapterCompletion {
                    sender: completion_sender,
                },
            );
            match result {
                Ok(mut active_call) => loop {
                    match completion_receiver.try_recv() {
                        Ok(result) => {
                            let _ignored =
                                completion_event_sender.send(AttemptEvent::Completed(result));
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            let _ignored = completion_event_sender.send(AttemptEvent::Completed(
                                Err(AdapterFailure {
                                    class: TranslationFailureClass::Unknown,
                                    retryable: false,
                                    retry_after: None,
                                    request_outcome_ambiguous: false,
                                }),
                            ));
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    }
                    match command_receiver.recv_timeout(WORKER_POLL_INTERVAL) {
                        Ok(AttemptCommand::Cancel) => {
                            let status = active_call.cancel();
                            let _ignored =
                                completion_event_sender.send(AttemptEvent::Cancelled(status));
                            break;
                        }
                        Ok(AttemptCommand::Begin) => {}
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                },
                Err(error) => {
                    let _ignored = event_sender.send(AttemptEvent::Completed(Err(error)));
                }
            }
        });
    let Ok(attempt_thread) = attempt_thread else {
        return unknown_attempt_failure();
    };
    if !shared.authorize_attempt_start(&command_sender) {
        drop(command_sender);
        let joined = attempt_thread.join().is_ok();
        if shared.stopped.load(Ordering::SeqCst) {
            return AttemptResult::Stopped;
        }
        if !joined {
            return unknown_attempt_failure();
        }
        return unknown_attempt_failure();
    }

    let mut attempt_thread = Some(attempt_thread);
    let mut finish_joined = |result| {
        let joined = attempt_thread
            .take()
            .is_some_and(|attempt| attempt.join().is_ok());
        if joined {
            result
        } else {
            unknown_attempt_failure()
        }
    };

    let mut cancellation_requested = false;
    loop {
        if shared.stopped.load(Ordering::SeqCst) {
            let _ignored = command_sender.try_send(AttemptCommand::Cancel);
            // Stop owns reservation cleanup and must not wait on provider code.
            // The bounded Adapter contract lets the attempt thread converge;
            // any later callback is disconnected and cannot publish an outcome.
            return AttemptResult::Stopped;
        }
        let now = clock.now();
        if !cancellation_requested && now >= deadline {
            let _ignored = command_sender.try_send(AttemptCommand::Cancel);
            cancellation_requested = true;
        }
        if cancellation_requested && now >= total_deadline {
            // A provider that violated the bounded cancel contract is
            // quarantined: dropping its JoinHandle detaches that attempt, while
            // the owner closes admission and starts no replacement request.
            return AttemptResult::UnconfirmedTimeout;
        }
        match event_receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(AttemptEvent::Completed(result)) => {
                return finish_joined(AttemptResult::Returned(result));
            }
            Ok(AttemptEvent::Cancelled(CancellationStatus::Confirmed)) => {
                return finish_joined(AttemptResult::Cancelled);
            }
            Ok(AttemptEvent::Cancelled(CancellationStatus::Unconfirmed)) => {
                return finish_joined(AttemptResult::UnconfirmedTimeout);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return finish_joined(unknown_attempt_failure()),
        }
    }
}

fn unknown_attempt_failure() -> AttemptResult {
    AttemptResult::Returned(Err(AdapterFailure {
        class: TranslationFailureClass::Unknown,
        retryable: false,
        retry_after: None,
        request_outcome_ambiguous: false,
    }))
}

fn finish_success(
    shared: &TranslationShared,
    job_id: u64,
    text: String,
    target: TranslationTarget,
) {
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    if shared.stopped.load(Ordering::SeqCst) || !state.sender_alive {
        return;
    }
    let Some(mut active) = state.active.take().filter(|active| active.job_id == job_id) else {
        return;
    };
    let Some(reservation) = active.reservation.take() else {
        return;
    };
    let outcome = TranslationTerminalOutcome::Completed(Box::new(CompletedTranslation {
        source_ref: active.source_ref,
        reservation,
        text,
        target,
        _permit: active.permit,
    }));
    state
        .outcomes
        .push_back(TranslationOutcomeEnvelope { outcome });
    drop(state);
    shared.wake.notify_all();
}

fn finish_failure(shared: &TranslationShared, job_id: u64, class: TranslationFailureClass) {
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    if shared.stopped.load(Ordering::SeqCst) || !state.sender_alive {
        return;
    }
    let Some(mut active) = state.active.take().filter(|active| active.job_id == job_id) else {
        return;
    };
    let Some(reservation) = active.reservation.take() else {
        return;
    };
    let outcome = TranslationTerminalOutcome::Failed(FailedTranslation {
        source_ref: active.source_ref,
        class,
        reservation: Box::new(reservation),
        _permit: active.permit,
    });
    state
        .outcomes
        .push_back(TranslationOutcomeEnvelope { outcome });
    drop(state);
    shared.wake.notify_all();
}

fn fail_closed_after_unconfirmed_timeout(shared: &TranslationShared) {
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    state.accepting = false;
    let mut failed = VecDeque::new();
    if let Some(active) = state.active.take() {
        failed.push_back(active);
    }
    failed.append(&mut state.pending);
    for mut job in failed {
        let Some(reservation) = job.reservation.take() else {
            continue;
        };
        let outcome = TranslationTerminalOutcome::Failed(FailedTranslation {
            source_ref: job.source_ref,
            class: TranslationFailureClass::DeadlineExceeded,
            reservation: Box::new(reservation),
            _permit: job.permit,
        });
        state
            .outcomes
            .push_back(TranslationOutcomeEnvelope { outcome });
    }
    drop(state);
    shared.wake.notify_all();
}

fn source_ref(reservation: &ReservedCompletedSource) -> Option<TranslationSourceRef> {
    let source = reservation.source();
    Some(TranslationSourceRef {
        generation: source.generation,
        stream_id: source.stream_id.clone(),
        unit_id: source.unit_id.clone()?,
        revision: source.revision,
    })
}

fn translation_language_tag(target: TranslationTarget) -> &'static str {
    match target {
        TranslationTarget::English => "en",
        TranslationTarget::SimplifiedChinese => "zh-Hans",
    }
}

fn saturating_add(base: Duration, delta: Duration) -> Duration {
    base.checked_add(delta).unwrap_or(Duration::MAX)
}

#[cfg(test)]
#[path = "translation/translation_tests.rs"]
mod tests;
