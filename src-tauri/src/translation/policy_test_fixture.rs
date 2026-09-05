use super::super::*;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const FIXTURE_WATCHDOG: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FixtureErrorKind {
    UnknownSource,
    MissingScript,
    UnknownAttempt,
    WrongAttemptPhase,
    UnusedScripts,
    AttemptsStillActive,
    DelaysStillActive,
    WatchdogExpired,
    Poisoned,
    AdmissionRejected,
    AlreadyFinished,
    WrongOwner,
    OwnerAlreadyBound,
    ModuleStartFailed,
}

/// A provider-neutral, redacted scenario error. Opaque attempt numbers and
/// closed enum values are safe to report; Source/Translation text never is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FixtureError {
    operation: &'static str,
    attempt: Option<AttemptId>,
    kind: FixtureErrorKind,
}

impl FixtureError {
    const fn new(operation: &'static str, kind: FixtureErrorKind) -> Self {
        Self {
            operation,
            attempt: None,
            kind,
        }
    }

    const fn for_attempt(
        operation: &'static str,
        attempt: AttemptId,
        kind: FixtureErrorKind,
    ) -> Self {
        Self {
            operation,
            attempt: Some(attempt),
            kind,
        }
    }

    pub(super) const fn kind(&self) -> FixtureErrorKind {
        self.kind
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Translation fixture operation {} failed: {:?}",
            self.operation, self.kind
        )?;
        if let Some(attempt) = self.attempt {
            write!(formatter, " ({attempt:?})")?;
        }
        Ok(())
    }
}

impl Error for FixtureError {}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FixtureIdentity(u64);

static NEXT_FIXTURE_IDENTITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_fixture_identity() -> FixtureIdentity {
    let sequence = NEXT_FIXTURE_IDENTITY
        .fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |current| current.checked_add(1),
        )
        .unwrap_or_else(|_| {
            std::panic::resume_unwind(Box::new("Translation fixture identity space exhausted"))
        });
    FixtureIdentity(sequence)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct AttemptId {
    fixture: FixtureIdentity,
    sequence: u64,
}

impl AttemptId {
    fn belongs_to(self, fixture: FixtureIdentity) -> bool {
        self.fixture == fixture
    }
}

impl fmt::Debug for AttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttemptId")
            .field("fixture", &self.fixture.0)
            .field("sequence", &self.sequence)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SourceId {
    fixture: FixtureIdentity,
    sequence: u64,
}

impl fmt::Debug for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceId")
            .field("fixture", &self.fixture.0)
            .field("sequence", &self.sequence)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AttemptTerminal {
    Completed,
    Failed,
    CancelledConfirmed,
    CancelledUnconfirmed,
    ReleasedAfterStop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AttemptRecord {
    id: AttemptId,
    source_id: SourceId,
    source_ref: TranslationSourceRef,
    target: TranslationTarget,
    attempt_number: u8,
    attempt_budget: Duration,
    total_budget: Duration,
    started_at: Duration,
    finished_at: Option<Duration>,
    terminal: Option<AttemptTerminal>,
    cancellation: Option<CancellationStatus>,
    entered: bool,
    quiesced: bool,
}

impl AttemptRecord {
    pub(super) const fn id(&self) -> AttemptId {
        self.id
    }

    pub(super) const fn source_ref(&self) -> &TranslationSourceRef {
        &self.source_ref
    }

    pub(super) const fn source_id(&self) -> SourceId {
        self.source_id
    }

    pub(super) const fn target(&self) -> TranslationTarget {
        self.target
    }

    pub(super) const fn attempt_number(&self) -> u8 {
        self.attempt_number
    }

    pub(super) const fn attempt_budget(&self) -> Duration {
        self.attempt_budget
    }

    pub(super) const fn total_budget(&self) -> Duration {
        self.total_budget
    }

    pub(super) const fn started_at(&self) -> Duration {
        self.started_at
    }

    pub(super) const fn finished_at(&self) -> Option<Duration> {
        self.finished_at
    }

    pub(super) const fn terminal(&self) -> Option<AttemptTerminal> {
        self.terminal
    }

    pub(super) const fn cancellation(&self) -> Option<CancellationStatus> {
        self.cancellation
    }

    pub(super) const fn is_quiesced(&self) -> bool {
        self.quiesced
    }
}

#[derive(Clone, Copy)]
struct FailureSpec {
    class: TranslationFailureClass,
    retryable: bool,
    retry_after: Option<Duration>,
    request_outcome_ambiguous: bool,
}

impl FailureSpec {
    const fn into_adapter_failure(self) -> AdapterFailure {
        AdapterFailure {
            class: self.class,
            retryable: self.retryable,
            retry_after: self.retry_after,
            request_outcome_ambiguous: self.request_outcome_ambiguous,
        }
    }
}

enum Resolution {
    Success(String),
    Failure(FailureSpec),
}

impl Resolution {
    fn into_adapter_result(self) -> Result<String, AdapterFailure> {
        match self {
            Self::Success(text) => Ok(text),
            Self::Failure(failure) => Err(failure.into_adapter_failure()),
        }
    }

    const fn terminal(&self) -> AttemptTerminal {
        match self {
            Self::Success(_) => AttemptTerminal::Completed,
            Self::Failure(_) => AttemptTerminal::Failed,
        }
    }
}

enum AttemptPlan {
    Immediate(Resolution),
    Held { cancellation: CancellationStatus },
    NonCooperativeBegin(Resolution),
}

enum PublishedAttempt {
    Immediate {
        id: AttemptId,
        resolution: Resolution,
        completion: AdapterCompletion,
    },
    Held {
        id: AttemptId,
    },
    NonCooperative {
        id: AttemptId,
    },
}

pub(super) struct AttemptScript(AttemptPlan);

impl AttemptScript {
    pub(super) fn success(text: impl Into<String>) -> Self {
        Self(AttemptPlan::Immediate(Resolution::Success(text.into())))
    }

    pub(super) const fn failure(
        class: TranslationFailureClass,
        retryable: bool,
        retry_after: Option<Duration>,
        request_outcome_ambiguous: bool,
    ) -> Self {
        Self(AttemptPlan::Immediate(Resolution::Failure(FailureSpec {
            class,
            retryable,
            retry_after,
            request_outcome_ambiguous,
        })))
    }

    pub(super) const fn held_confirmed() -> Self {
        Self(AttemptPlan::Held {
            cancellation: CancellationStatus::Confirmed,
        })
    }

    pub(super) const fn held_unconfirmed() -> Self {
        Self(AttemptPlan::Held {
            cancellation: CancellationStatus::Unconfirmed,
        })
    }

    pub(super) fn non_cooperative_success(text: impl Into<String>) -> Self {
        Self(AttemptPlan::NonCooperativeBegin(Resolution::Success(
            text.into(),
        )))
    }

    pub(super) const fn non_cooperative_failure(
        class: TranslationFailureClass,
        retryable: bool,
        retry_after: Option<Duration>,
        request_outcome_ambiguous: bool,
    ) -> Self {
        Self(AttemptPlan::NonCooperativeBegin(Resolution::Failure(
            FailureSpec {
                class,
                retryable,
                retry_after,
                request_outcome_ambiguous,
            },
        )))
    }
}

struct RegisteredSource {
    source_ref: TranslationSourceRef,
    text: String,
}

struct PlannedAttempt {
    source_id: SourceId,
    plan: AttemptPlan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptPhase {
    HeldPending { cancellation: CancellationStatus },
    Resolving,
    CancelledUnconfirmed,
    NonCooperativeBlocked,
    NonCooperativeReleaseRequested,
    Quiesced,
}

struct ActiveAttempt {
    completion: Option<AdapterCompletion>,
    phase: AttemptPhase,
    non_cooperative_resolution: Option<Resolution>,
}

struct FixtureState {
    scripts: VecDeque<PlannedAttempt>,
    sources: HashMap<SourceId, RegisteredSource>,
    records: Vec<AttemptRecord>,
    attempts: HashMap<AttemptId, ActiveAttempt>,
    next_source: u64,
    next_attempt: u64,
    first_error: Option<FixtureError>,
    cleaning_up: bool,
    finished: bool,
    owner_starting: bool,
    owner: Option<TranslationOwnerRegistration>,
    adapter_alive: bool,
    adapter_calls_inflight: usize,
    begin_inflight: usize,
    active_calls: usize,
    delay_calls_inflight: usize,
    pause_before_begin_guard: bool,
    before_begin_guard_paused: bool,
    pause_after_begin_guard: bool,
    begin_guard_paused: bool,
    pause_after_attempt_publication: bool,
    attempt_publication_paused: bool,
    finish_waiting_for_adapter: bool,
    drop_waiting_for_adapter: bool,
}

impl FixtureState {
    const fn is_closed(&self) -> bool {
        self.cleaning_up || self.finished
    }
}

struct FixtureShared {
    identity: FixtureIdentity,
    state: Mutex<FixtureState>,
    changed: Condvar,
    clock: Arc<ScenarioClock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DelayRecord {
    requested: Duration,
    started_at: Duration,
    finished_at: Option<Duration>,
}

impl DelayRecord {
    pub(super) const fn requested(&self) -> Duration {
        self.requested
    }

    pub(super) const fn started_at(&self) -> Duration {
        self.started_at
    }

    pub(super) const fn finished_at(&self) -> Option<Duration> {
        self.finished_at
    }
}

#[derive(Default)]
struct ClockState {
    now: Duration,
    delays: Vec<DelayRecord>,
    active_delays: usize,
    cancelled: bool,
    first_error: Option<FixtureError>,
    waiting_for_delay_count: Option<usize>,
}

#[derive(Default)]
struct ScenarioClock {
    state: Mutex<ClockState>,
    changed: Condvar,
}

impl TranslationClock for ScenarioClock {
    fn now(&self) -> Duration {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .now
    }
}

struct ScenarioDelay {
    shared: Arc<FixtureShared>,
}

struct DelayCallGuard {
    shared: Arc<FixtureShared>,
}

impl DelayCallGuard {
    fn enter(shared: &Arc<FixtureShared>) -> Option<Self> {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed() {
            return None;
        }
        state.delay_calls_inflight = state.delay_calls_inflight.saturating_add(1);
        shared.changed.notify_all();
        Some(Self {
            shared: Arc::clone(shared),
        })
    }
}

impl Drop for DelayCallGuard {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.delay_calls_inflight = state.delay_calls_inflight.saturating_sub(1);
        self.shared.changed.notify_all();
    }
}

impl CancellableDelay for ScenarioDelay {
    fn wait(
        &self,
        duration: Duration,
        stopped: &AtomicBool,
        _clock: &dyn TranslationClock,
    ) -> bool {
        let Some(delay_call) = DelayCallGuard::enter(&self.shared) else {
            return false;
        };
        let mut state = self
            .shared
            .clock
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let started_at = state.now;
        let target = saturating_add(started_at, duration);
        let delay_index = state.delays.len();
        state.delays.push(DelayRecord {
            requested: duration,
            started_at,
            finished_at: None,
        });
        state.active_delays = state.active_delays.saturating_add(1);
        self.shared.clock.changed.notify_all();
        while state.now < target && !state.cancelled && !stopped.load(Ordering::SeqCst) {
            state = match self.shared.clock.changed.wait(state) {
                Ok(next) => next,
                Err(poisoned) => {
                    let mut next = poisoned.into_inner();
                    next.cancelled = true;
                    next
                }
            };
        }
        let completed = state.now >= target && !state.cancelled && !stopped.load(Ordering::SeqCst);
        let now = state.now;
        if let Some(record) = state.delays.get_mut(delay_index) {
            record.finished_at = Some(now);
        }
        state.active_delays = state.active_delays.saturating_sub(1);
        self.shared.clock.changed.notify_all();
        // Preserve the fixture's only two-lock order: FixtureState → ClockState.
        // DelayCallGuard locks FixtureState, so release ClockState explicitly.
        drop(state);
        drop(delay_call);
        completed
    }

    fn cancel(&self) {
        let mut state = self
            .shared
            .clock
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cancelled = true;
        self.shared.clock.changed.notify_all();
    }
}

struct FixedScenarioJitter(Duration);

impl RetryJitter for FixedScenarioJitter {
    fn delay(&self, _base: Duration) -> Duration {
        self.0
    }
}

struct FixtureAdapter {
    shared: Arc<FixtureShared>,
}

impl Drop for FixtureAdapter {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.adapter_alive = false;
        self.shared.changed.notify_all();
    }
}

struct BeginInflightGuard {
    shared: Arc<FixtureShared>,
}

struct AdapterCallGuard {
    shared: Arc<FixtureShared>,
}

impl AdapterCallGuard {
    fn enter(shared: &Arc<FixtureShared>) -> Result<Self, AdapterFailure> {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed() {
            return Err(fixture_adapter_failure());
        }
        state.adapter_calls_inflight = state.adapter_calls_inflight.saturating_add(1);
        shared.changed.notify_all();
        Ok(Self {
            shared: Arc::clone(shared),
        })
    }
}

impl Drop for AdapterCallGuard {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.adapter_calls_inflight = state.adapter_calls_inflight.saturating_sub(1);
        self.shared.changed.notify_all();
    }
}

impl BeginInflightGuard {
    fn enter(shared: &Arc<FixtureShared>) -> Result<Self, AdapterFailure> {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed() {
            return Err(fixture_adapter_failure());
        }
        state.begin_inflight = state.begin_inflight.saturating_add(1);
        Ok(Self {
            shared: Arc::clone(shared),
        })
    }
}

impl Drop for BeginInflightGuard {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.begin_inflight = state.begin_inflight.saturating_sub(1);
        self.shared.changed.notify_all();
    }
}

impl CompletedTextAdapter for FixtureAdapter {
    fn begin(
        &self,
        request: CompletedTextRequest,
        control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        let _adapter_call = AdapterCallGuard::enter(&self.shared)?;
        self.pause_before_begin_guard();
        let _begin_inflight = BeginInflightGuard::enter(&self.shared)?;
        let started_at = self.shared.clock.now();
        let published = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| fixture_adapter_failure())?;
            if state.pause_after_begin_guard {
                state.begin_guard_paused = true;
                self.shared.changed.notify_all();
                while state.pause_after_begin_guard && !state.is_closed() {
                    state = self
                        .shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
            if state.is_closed() {
                return Err(fixture_adapter_failure());
            }
            let Some(planned) = state.scripts.pop_front() else {
                store_error(
                    &self.shared,
                    &mut state,
                    FixtureError::new("begin", FixtureErrorKind::MissingScript),
                );
                self.shared.changed.notify_all();
                return Err(fixture_adapter_failure());
            };
            let Some(registered) = state.sources.get(&planned.source_id) else {
                store_error(
                    &self.shared,
                    &mut state,
                    FixtureError::new("begin", FixtureErrorKind::UnknownSource),
                );
                self.shared.changed.notify_all();
                return Err(fixture_adapter_failure());
            };
            if registered.text != request.source_text {
                store_error(
                    &self.shared,
                    &mut state,
                    FixtureError::new("begin", FixtureErrorKind::UnknownSource),
                );
                self.shared.changed.notify_all();
                return Err(fixture_adapter_failure());
            }
            let source_ref = registered.source_ref.clone();
            state.next_attempt = state.next_attempt.saturating_add(1);
            let id = AttemptId {
                fixture: self.shared.identity,
                sequence: state.next_attempt,
            };
            let attempt_number = u8::try_from(
                state
                    .records
                    .iter()
                    .filter(|record| record.source_id == planned.source_id)
                    .count()
                    .saturating_add(1),
            )
            .unwrap_or(u8::MAX);
            let record = AttemptRecord {
                id,
                source_id: planned.source_id,
                source_ref,
                target: request.target,
                attempt_number,
                attempt_budget: control.attempt_budget,
                total_budget: control.total_budget,
                started_at,
                finished_at: None,
                terminal: None,
                cancellation: None,
                entered: true,
                quiesced: false,
            };
            let published = match planned.plan {
                AttemptPlan::Immediate(resolution) => PublishedAttempt::Immediate {
                    id,
                    resolution,
                    completion,
                },
                AttemptPlan::Held { cancellation } => {
                    state.attempts.insert(
                        id,
                        ActiveAttempt {
                            completion: Some(completion),
                            phase: AttemptPhase::HeldPending { cancellation },
                            non_cooperative_resolution: None,
                        },
                    );
                    PublishedAttempt::Held { id }
                }
                AttemptPlan::NonCooperativeBegin(resolution) => {
                    state.attempts.insert(
                        id,
                        ActiveAttempt {
                            completion: Some(completion),
                            phase: AttemptPhase::NonCooperativeBlocked,
                            non_cooperative_resolution: Some(resolution),
                        },
                    );
                    PublishedAttempt::NonCooperative { id }
                }
            };
            state.records.push(record);
            self.shared.changed.notify_all();
            published
        };
        self.pause_after_attempt_publication();

        match published {
            PublishedAttempt::Immediate {
                id,
                resolution,
                completion,
            } => {
                let terminal = resolution.terminal();
                completion.finish(resolution.into_adapter_result());
                finish_attempt(&self.shared, id, terminal);
                Ok(Box::new(FixtureActiveCall::new(
                    id,
                    Arc::clone(&self.shared),
                )))
            }
            PublishedAttempt::Held { id } => Ok(Box::new(FixtureActiveCall::new(
                id,
                Arc::clone(&self.shared),
            ))),
            PublishedAttempt::NonCooperative { id } => {
                // Publication transferred callback/blocker cleanup to this
                // branch. Poisoning must not abandon that terminal transition.
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                loop {
                    let should_leave = state.is_closed()
                        || state.attempts.get(&id).is_some_and(|attempt| {
                            attempt.phase == AttemptPhase::NonCooperativeReleaseRequested
                        });
                    if should_leave {
                        break;
                    }
                    state = self
                        .shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                let cleaning_up = state.is_closed();
                let (completion, resolution) = if let Some(attempt) = state.attempts.get_mut(&id) {
                    attempt.phase = AttemptPhase::Resolving;
                    (
                        attempt.completion.take(),
                        attempt.non_cooperative_resolution.take(),
                    )
                } else {
                    store_error(
                        &self.shared,
                        &mut state,
                        FixtureError::for_attempt(
                            "release-non-cooperative",
                            id,
                            FixtureErrorKind::UnknownAttempt,
                        ),
                    );
                    (None, None)
                };
                drop(state);
                let terminal = if cleaning_up {
                    AttemptTerminal::ReleasedAfterStop
                } else if let (Some(completion), Some(resolution)) = (completion, resolution) {
                    let terminal = resolution.terminal();
                    completion.finish(resolution.into_adapter_result());
                    terminal
                } else {
                    self.store_attempt_error(id, "release-non-cooperative");
                    AttemptTerminal::ReleasedAfterStop
                };
                finish_attempt(&self.shared, id, terminal);
                Ok(Box::new(FixtureActiveCall::new(
                    id,
                    Arc::clone(&self.shared),
                )))
            }
        }
    }
}

impl FixtureAdapter {
    fn pause_before_begin_guard(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.pause_before_begin_guard {
            return;
        }
        state.before_begin_guard_paused = true;
        self.shared.changed.notify_all();
        // Finish deliberately waits for the real Adapter object's final Drop,
        // so this test gate only opens explicitly or for failure-safe cleanup.
        while state.pause_before_begin_guard && !state.cleaning_up {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn pause_after_attempt_publication(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.pause_after_attempt_publication {
            return;
        }
        state.attempt_publication_paused = true;
        self.shared.changed.notify_all();
        while state.pause_after_attempt_publication && !state.is_closed() {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn store_attempt_error(&self, id: AttemptId, operation: &'static str) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        store_error(
            &self.shared,
            &mut state,
            FixtureError::for_attempt(operation, id, FixtureErrorKind::WrongAttemptPhase),
        );
        self.shared.changed.notify_all();
    }
}

fn finish_attempt(shared: &Arc<FixtureShared>, id: AttemptId, terminal: AttemptTerminal) {
    let finished_at = shared.clock.now();
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(attempt) = state.attempts.get_mut(&id) {
        attempt.phase = AttemptPhase::Quiesced;
    }
    if let Some(record) = state.records.iter_mut().find(|record| record.id == id) {
        record.entered = true;
        record.finished_at = Some(finished_at);
        record.terminal = Some(terminal);
        record.quiesced = true;
    }
    shared.changed.notify_all();
}

struct FixtureActiveCall {
    id: AttemptId,
    shared: Arc<FixtureShared>,
}

impl FixtureActiveCall {
    fn new(id: AttemptId, shared: Arc<FixtureShared>) -> Self {
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.active_calls = state.active_calls.saturating_add(1);
            shared.changed.notify_all();
        }
        Self { id, shared }
    }
}

impl ActiveTranslationCall for FixtureActiveCall {
    fn cancel(&mut self) -> CancellationStatus {
        let finished_at = self.shared.clock.now();
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                store_error(
                    &self.shared,
                    &mut state,
                    FixtureError::for_attempt("cancel", self.id, FixtureErrorKind::Poisoned),
                );
                state
            }
        };
        let Some(attempt) = state.attempts.get_mut(&self.id) else {
            // An immediate completion has already quiesced and its ActiveCall
            // can be dropped without a second cancellation transition.
            return CancellationStatus::Confirmed;
        };
        let status = match attempt.phase {
            AttemptPhase::HeldPending { cancellation } => {
                if attempt.completion.is_none() {
                    let error = FixtureError::for_attempt(
                        "cancel",
                        self.id,
                        FixtureErrorKind::WrongAttemptPhase,
                    );
                    store_error(&self.shared, &mut state, error);
                    self.shared.changed.notify_all();
                    return CancellationStatus::Unconfirmed;
                }
                if cancellation == CancellationStatus::Confirmed {
                    attempt.completion.take();
                    attempt.phase = AttemptPhase::Quiesced;
                } else {
                    attempt.phase = AttemptPhase::CancelledUnconfirmed;
                }
                cancellation
            }
            AttemptPhase::Resolving => {
                if let Some(record) = state.records.iter_mut().find(|record| record.id == self.id) {
                    record.cancellation = Some(CancellationStatus::Unconfirmed);
                }
                self.shared.changed.notify_all();
                return CancellationStatus::Unconfirmed;
            }
            AttemptPhase::CancelledUnconfirmed
            | AttemptPhase::NonCooperativeBlocked
            | AttemptPhase::NonCooperativeReleaseRequested
            | AttemptPhase::Quiesced => {
                let error = FixtureError::for_attempt(
                    "cancel",
                    self.id,
                    FixtureErrorKind::WrongAttemptPhase,
                );
                store_error(&self.shared, &mut state, error);
                self.shared.changed.notify_all();
                return CancellationStatus::Unconfirmed;
            }
        };
        if let Some(record) = state.records.iter_mut().find(|record| record.id == self.id) {
            record.finished_at = Some(finished_at);
            record.cancellation = Some(status);
            record.terminal = Some(match status {
                CancellationStatus::Confirmed => AttemptTerminal::CancelledConfirmed,
                CancellationStatus::Unconfirmed => AttemptTerminal::CancelledUnconfirmed,
            });
            record.quiesced = status == CancellationStatus::Confirmed;
        }
        self.shared.changed.notify_all();
        status
    }
}

impl Drop for FixtureActiveCall {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active_calls = state.active_calls.saturating_sub(1);
        self.shared.changed.notify_all();
    }
}

fn fixture_adapter_failure() -> AdapterFailure {
    AdapterFailure {
        class: TranslationFailureClass::Unknown,
        retryable: false,
        retry_after: None,
        request_outcome_ambiguous: false,
    }
}

fn store_error(shared: &FixtureShared, state: &mut FixtureState, error: FixtureError) {
    // The only permitted two-lock order is FixtureState → ClockState. Clock
    // paths never call this helper, and ScenarioDelay explicitly releases its
    // ClockState guard before DelayCallGuard touches FixtureState.
    let first_error = *state.first_error.get_or_insert(error);
    {
        let mut clock = shared
            .clock
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clock.first_error.get_or_insert(first_error);
    }
    shared.changed.notify_all();
    shared.clock.changed.notify_all();
}

fn module_is_registered(state: &FixtureState, module: &FixtureModule) -> bool {
    state
        .owner
        .as_ref()
        .is_some_and(|owner| owner.matches_module(&module.inner))
}

fn attempt_is_fully_quiesced(state: &FixtureState, id: AttemptId) -> Option<bool> {
    let record = state.records.iter().find(|record| record.id == id)?;
    Some(
        record.quiesced
            && record.terminal.is_some()
            && state.adapter_calls_inflight == 0
            && state.begin_inflight == 0
            && state.active_calls == 0
            && state
                .attempts
                .get(&id)
                .is_none_or(|attempt| attempt.phase == AttemptPhase::Quiesced),
    )
}

fn all_attempts_fully_quiesced(state: &FixtureState) -> bool {
    state
        .records
        .iter()
        .all(|record| record.quiesced && record.terminal.is_some())
        && state.adapter_calls_inflight == 0
        && state.begin_inflight == 0
        && state.active_calls == 0
        && state
            .attempts
            .values()
            .all(|attempt| attempt.phase == AttemptPhase::Quiesced)
}

fn all_fixture_lifetimes_quiesced(state: &FixtureState) -> bool {
    !state.owner_starting
        && !state.adapter_alive
        && all_attempts_fully_quiesced(state)
        && state.delay_calls_inflight == 0
}

fn stopped_owner_lifetimes_drained(state: &FixtureState) -> bool {
    !state.owner_starting
        && !state.adapter_alive
        && state.adapter_calls_inflight == 0
        && state.begin_inflight == 0
        && state.active_calls == 0
        && state.delay_calls_inflight == 0
}

pub(super) struct TranslationPolicyFixture {
    shared: Arc<FixtureShared>,
}

/// A Translation Module whose adapter and policy dependencies were bound by
/// one fixture. Keeping construction here prevents tests from mixing an
/// adapter, clock, delay, or owner proof from different scenarios.
pub(super) struct FixtureModule {
    inner: TranslationModule,
}

impl FixtureModule {
    pub(super) fn try_submit(
        &self,
        reservation: ReservedCompletedSource,
    ) -> Result<(), TranslationSubmissionRejection> {
        self.inner.try_submit(reservation)
    }

    pub(super) fn try_submit_with_stop_hook(
        &self,
        reservation: ReservedCompletedSource,
    ) -> Result<(), TranslationSubmissionRejection> {
        let shared = Arc::clone(&self.inner.shared);
        self.inner
            .try_submit_with_hook(reservation, move || shared.request_stop())
    }

    pub(super) fn stop_and_confirm_owner_quiesced(
        &mut self,
    ) -> AppResult<TranslationOwnerQuiesced> {
        self.inner.stop_and_confirm_owner_quiesced()
    }
}

impl TranslationPolicyFixture {
    pub(super) fn new() -> Self {
        let clock = Arc::new(ScenarioClock::default());
        let identity = next_fixture_identity();
        Self {
            shared: Arc::new(FixtureShared {
                identity,
                state: Mutex::new(FixtureState {
                    scripts: VecDeque::new(),
                    sources: HashMap::new(),
                    records: Vec::new(),
                    attempts: HashMap::new(),
                    next_source: 0,
                    next_attempt: 0,
                    first_error: None,
                    cleaning_up: false,
                    finished: false,
                    owner_starting: false,
                    owner: None,
                    adapter_alive: false,
                    adapter_calls_inflight: 0,
                    begin_inflight: 0,
                    active_calls: 0,
                    delay_calls_inflight: 0,
                    pause_before_begin_guard: false,
                    before_begin_guard_paused: false,
                    pause_after_begin_guard: false,
                    begin_guard_paused: false,
                    pause_after_attempt_publication: false,
                    attempt_publication_paused: false,
                    finish_waiting_for_adapter: false,
                    drop_waiting_for_adapter: false,
                }),
                changed: Condvar::new(),
                clock,
            }),
        }
    }

    pub(super) fn start_module(
        &self,
        target: TranslationTarget,
    ) -> Result<(FixtureModule, TranslationOutcomeReceiver), FixtureError> {
        self.start_module_with_jitter(target, Duration::from_millis(250))
    }

    pub(super) fn start_module_with_jitter(
        &self,
        target: TranslationTarget,
        jitter: Duration,
    ) -> Result<(FixtureModule, TranslationOutcomeReceiver), FixtureError> {
        {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| FixtureError::new("start-module", FixtureErrorKind::Poisoned))?;
            if state.is_closed() {
                return Err(FixtureError::new(
                    "start-module",
                    FixtureErrorKind::AlreadyFinished,
                ));
            }
            if state.owner_starting || state.owner.is_some() {
                return Err(FixtureError::new(
                    "start-module",
                    FixtureErrorKind::OwnerAlreadyBound,
                ));
            }
            state.owner_starting = true;
            state.adapter_alive = true;
            self.shared.changed.notify_all();
        }

        // One-shot owner binding creates exactly one Adapter object. Its Drop
        // is the acknowledgement that no owner or detached attempt can invoke
        // this fixture again.
        let adapter = Arc::new(FixtureAdapter {
            shared: Arc::clone(&self.shared),
        }) as Arc<dyn CompletedTextAdapter>;
        let started = TranslationModule::start_for_test(
            target,
            adapter,
            self.dependencies_with_jitter(jitter),
        );
        let (mut inner, outcomes) = match started {
            Ok(started) => started,
            Err(_) => {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.owner_starting = false;
                self.shared.changed.notify_all();
                return Err(FixtureError::new(
                    "start-module",
                    FixtureErrorKind::ModuleStartFailed,
                ));
            }
        };
        let registration = inner.owner_registration();
        let accepted = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.owner_starting = false;
            if state.is_closed() {
                false
            } else {
                state.owner = Some(registration);
                true
            }
        };
        self.shared.changed.notify_all();
        if !accepted {
            let _ignored = inner.stop();
            return Err(FixtureError::new(
                "start-module",
                FixtureErrorKind::AlreadyFinished,
            ));
        }
        Ok((FixtureModule { inner }, outcomes))
    }

    fn dependencies_with_jitter(&self, jitter: Duration) -> TestPolicyDependencies {
        TestPolicyDependencies {
            clock: Arc::clone(&self.shared.clock) as Arc<dyn TranslationClock>,
            delay: Arc::new(ScenarioDelay {
                shared: Arc::clone(&self.shared),
            }),
            jitter: Arc::new(FixedScenarioJitter(jitter)),
        }
    }

    pub(super) fn admit(
        &self,
        module: &FixtureModule,
        reservation: ReservedCompletedSource,
        scripts: impl IntoIterator<Item = AttemptScript>,
    ) -> Result<SourceId, FixtureError> {
        {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| FixtureError::new("admit", FixtureErrorKind::Poisoned))?;
            if !module_is_registered(&state, module) {
                return Err(FixtureError::new("admit", FixtureErrorKind::WrongOwner));
            }
        }
        // Caller-provided iterators execute outside fixture critical sections;
        // a panicking scenario builder therefore cannot poison synchronization
        // needed to release an already-blocked attempt.
        let scripts: Vec<AttemptScript> = scripts.into_iter().collect();
        let source_ref = source_ref(&reservation)
            .ok_or_else(|| FixtureError::new("admit", FixtureErrorKind::UnknownSource))?;
        let source_text = reservation.source().text.clone();
        let source_id = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| FixtureError::new("admit", FixtureErrorKind::Poisoned))?;
            if !module_is_registered(&state, module) {
                return Err(FixtureError::new("admit", FixtureErrorKind::WrongOwner));
            }
            if state.is_closed() {
                let error = FixtureError::new("admit", FixtureErrorKind::AlreadyFinished);
                store_error(&self.shared, &mut state, error);
                self.shared.changed.notify_all();
                return Err(error);
            }
            state.next_source = state.next_source.saturating_add(1);
            let source_id = SourceId {
                fixture: self.shared.identity,
                sequence: state.next_source,
            };
            state.sources.insert(
                source_id,
                RegisteredSource {
                    source_ref,
                    text: source_text,
                },
            );
            state
                .scripts
                .extend(scripts.into_iter().map(|script| PlannedAttempt {
                    source_id,
                    plan: script.0,
                }));
            source_id
        };
        if module.try_submit(reservation).is_ok() {
            return Ok(source_id);
        }
        if let Ok(mut state) = self.shared.state.lock() {
            state.sources.remove(&source_id);
            state
                .scripts
                .retain(|planned| planned.source_id != source_id);
        }
        Err(FixtureError::new(
            "admit",
            FixtureErrorKind::AdmissionRejected,
        ))
    }

    fn require_attempt_owner(
        &self,
        id: AttemptId,
        operation: &'static str,
    ) -> Result<(), FixtureError> {
        if id.belongs_to(self.shared.identity) {
            Ok(())
        } else {
            Err(FixtureError::for_attempt(
                operation,
                id,
                FixtureErrorKind::WrongOwner,
            ))
        }
    }

    pub(super) fn wait_for_attempt_count(
        &self,
        expected: usize,
        watchdog: Duration,
    ) -> Result<Vec<AttemptRecord>, FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| FixtureError::new("wait-attempt", FixtureErrorKind::Poisoned))?;
        loop {
            if let Some(error) = state.first_error {
                return Err(error);
            }
            let entered = state.records.iter().filter(|record| record.entered).count();
            if entered >= expected {
                return Ok(state.records.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "wait-attempt",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| FixtureError::new("wait-attempt", FixtureErrorKind::Poisoned))?;
            state = next;
            if wait.timed_out() {
                continue;
            }
        }
    }

    pub(super) fn complete(
        &self,
        id: AttemptId,
        text: impl Into<String>,
    ) -> Result<(), FixtureError> {
        self.resolve(id, Resolution::Success(text.into()))
    }

    pub(super) fn fail(
        &self,
        id: AttemptId,
        class: TranslationFailureClass,
        retryable: bool,
        retry_after: Option<Duration>,
        request_outcome_ambiguous: bool,
    ) -> Result<(), FixtureError> {
        self.resolve(
            id,
            Resolution::Failure(FailureSpec {
                class,
                retryable,
                retry_after,
                request_outcome_ambiguous,
            }),
        )
    }

    fn resolve(&self, id: AttemptId, resolution: Resolution) -> Result<(), FixtureError> {
        self.require_attempt_owner(id, "resolve")?;
        let terminal = resolution.terminal();
        let completion = {
            let mut state = self.shared.state.lock().map_err(|_| {
                FixtureError::for_attempt("resolve", id, FixtureErrorKind::Poisoned)
            })?;
            if state.is_closed() {
                let error =
                    FixtureError::for_attempt("resolve", id, FixtureErrorKind::AlreadyFinished);
                store_error(&self.shared, &mut state, error);
                self.shared.changed.notify_all();
                return Err(error);
            }
            let Some(attempt) = state.attempts.get_mut(&id) else {
                let error =
                    FixtureError::for_attempt("resolve", id, FixtureErrorKind::UnknownAttempt);
                store_error(&self.shared, &mut state, error);
                self.shared.changed.notify_all();
                return Err(error);
            };
            if !matches!(
                attempt.phase,
                AttemptPhase::HeldPending { .. } | AttemptPhase::CancelledUnconfirmed
            ) {
                let error =
                    FixtureError::for_attempt("resolve", id, FixtureErrorKind::WrongAttemptPhase);
                store_error(&self.shared, &mut state, error);
                self.shared.changed.notify_all();
                return Err(error);
            }
            attempt.phase = AttemptPhase::Resolving;
            let Some(completion) = attempt.completion.take() else {
                let error =
                    FixtureError::for_attempt("resolve", id, FixtureErrorKind::WrongAttemptPhase);
                store_error(&self.shared, &mut state, error);
                self.shared.changed.notify_all();
                return Err(error);
            };
            completion
        };
        completion.finish(resolution.into_adapter_result());
        finish_attempt(&self.shared, id, terminal);
        Ok(())
    }

    pub(super) fn advance(&self, duration: Duration) -> Result<(), FixtureError> {
        {
            let mut fixture = self
                .shared
                .state
                .lock()
                .map_err(|_| FixtureError::new("advance-time", FixtureErrorKind::Poisoned))?;
            if fixture.is_closed() {
                let error = FixtureError::new("advance-time", FixtureErrorKind::AlreadyFinished);
                store_error(&self.shared, &mut fixture, error);
                self.shared.changed.notify_all();
                return Err(error);
            }
        }
        let mut state = self
            .shared
            .clock
            .state
            .lock()
            .map_err(|_| FixtureError::new("advance-time", FixtureErrorKind::Poisoned))?;
        state.now = saturating_add(state.now, duration);
        self.shared.clock.changed.notify_all();
        Ok(())
    }

    pub(super) fn wait_for_delay_count(
        &self,
        expected: usize,
        watchdog: Duration,
    ) -> Result<Vec<DelayRecord>, FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .clock
            .state
            .lock()
            .map_err(|_| FixtureError::new("wait-delay", FixtureErrorKind::Poisoned))?;
        loop {
            if let Some(error) = state.first_error {
                state.waiting_for_delay_count = None;
                return Err(error);
            }
            if state.delays.len() >= expected {
                state.waiting_for_delay_count = None;
                return Ok(state.delays.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.waiting_for_delay_count = None;
                return Err(FixtureError::new(
                    "wait-delay",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            state.waiting_for_delay_count = Some(expected);
            self.shared.clock.changed.notify_all();
            let (next, _wait) = self
                .shared
                .clock
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| FixtureError::new("wait-delay", FixtureErrorKind::Poisoned))?;
            state = next;
        }
    }

    fn wait_until_delay_waiting_for_count(
        &self,
        expected: usize,
        watchdog: Duration,
    ) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .clock
            .state
            .lock()
            .map_err(|_| FixtureError::new("wait-delay-waiter", FixtureErrorKind::Poisoned))?;
        loop {
            if let Some(error) = state.first_error {
                return Err(error);
            }
            if state.waiting_for_delay_count == Some(expected) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "wait-delay-waiter",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .clock
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| FixtureError::new("wait-delay-waiter", FixtureErrorKind::Poisoned))?;
            state = next;
        }
    }

    pub(super) fn release_non_cooperative(&self, id: AttemptId) -> Result<(), FixtureError> {
        self.require_attempt_owner(id, "release")?;
        let mut state =
            self.shared.state.lock().map_err(|_| {
                FixtureError::for_attempt("release", id, FixtureErrorKind::Poisoned)
            })?;
        if state.is_closed() {
            let error = FixtureError::for_attempt("release", id, FixtureErrorKind::AlreadyFinished);
            store_error(&self.shared, &mut state, error);
            self.shared.changed.notify_all();
            return Err(error);
        }
        let Some(attempt) = state.attempts.get_mut(&id) else {
            let error = FixtureError::for_attempt("release", id, FixtureErrorKind::UnknownAttempt);
            store_error(&self.shared, &mut state, error);
            self.shared.changed.notify_all();
            return Err(error);
        };
        if attempt.phase != AttemptPhase::NonCooperativeBlocked {
            let error =
                FixtureError::for_attempt("release", id, FixtureErrorKind::WrongAttemptPhase);
            store_error(&self.shared, &mut state, error);
            self.shared.changed.notify_all();
            return Err(error);
        }
        attempt.phase = AttemptPhase::NonCooperativeReleaseRequested;
        self.shared.changed.notify_all();
        Ok(())
    }

    pub(super) fn quiesce_unconfirmed(&self, id: AttemptId) -> Result<(), FixtureError> {
        self.require_attempt_owner(id, "quiesce")?;
        let mut state =
            self.shared.state.lock().map_err(|_| {
                FixtureError::for_attempt("quiesce", id, FixtureErrorKind::Poisoned)
            })?;
        if state.is_closed() {
            let error = FixtureError::for_attempt("quiesce", id, FixtureErrorKind::AlreadyFinished);
            store_error(&self.shared, &mut state, error);
            self.shared.changed.notify_all();
            return Err(error);
        }
        let Some(attempt) = state.attempts.get_mut(&id) else {
            let error = FixtureError::for_attempt("quiesce", id, FixtureErrorKind::UnknownAttempt);
            store_error(&self.shared, &mut state, error);
            self.shared.changed.notify_all();
            return Err(error);
        };
        if attempt.phase != AttemptPhase::CancelledUnconfirmed || attempt.completion.is_none() {
            let error =
                FixtureError::for_attempt("quiesce", id, FixtureErrorKind::WrongAttemptPhase);
            store_error(&self.shared, &mut state, error);
            self.shared.changed.notify_all();
            return Err(error);
        }
        attempt.completion.take();
        attempt.phase = AttemptPhase::Quiesced;
        if let Some(record) = state.records.iter_mut().find(|record| record.id == id) {
            record.quiesced = true;
        }
        self.shared.changed.notify_all();
        Ok(())
    }

    pub(super) fn wait_for_cancellation(
        &self,
        id: AttemptId,
        expected: CancellationStatus,
        watchdog: Duration,
    ) -> Result<AttemptRecord, FixtureError> {
        self.require_attempt_owner(id, "wait-cancellation")?;
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self.shared.state.lock().map_err(|_| {
            FixtureError::for_attempt("wait-cancellation", id, FixtureErrorKind::Poisoned)
        })?;
        loop {
            if let Some(error) = state.first_error {
                return Err(error);
            }
            let Some(record) = state.records.iter().find(|record| record.id == id) else {
                return Err(FixtureError::for_attempt(
                    "wait-cancellation",
                    id,
                    FixtureErrorKind::UnknownAttempt,
                ));
            };
            let phase_matches = state
                .attempts
                .get(&id)
                .is_some_and(|attempt| match expected {
                    CancellationStatus::Confirmed => attempt.phase == AttemptPhase::Quiesced,
                    CancellationStatus::Unconfirmed => {
                        attempt.phase == AttemptPhase::CancelledUnconfirmed
                    }
                });
            if record.cancellation == Some(expected) && phase_matches {
                return Ok(record.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::for_attempt(
                    "wait-cancellation",
                    id,
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) =
                self.shared
                    .changed
                    .wait_timeout(state, remaining)
                    .map_err(|_| {
                        FixtureError::for_attempt(
                            "wait-cancellation",
                            id,
                            FixtureErrorKind::Poisoned,
                        )
                    })?;
            state = next;
        }
    }

    pub(super) fn wait_for_quiescence(
        &self,
        id: AttemptId,
        watchdog: Duration,
    ) -> Result<AttemptRecord, FixtureError> {
        self.require_attempt_owner(id, "wait-quiescence")?;
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self.shared.state.lock().map_err(|_| {
            FixtureError::for_attempt("wait-quiescence", id, FixtureErrorKind::Poisoned)
        })?;
        loop {
            if let Some(error) = state.first_error {
                return Err(error);
            }
            match attempt_is_fully_quiesced(&state, id) {
                Some(true) => {
                    let record = state
                        .records
                        .iter()
                        .find(|record| record.id == id)
                        .cloned()
                        .ok_or_else(|| {
                            FixtureError::for_attempt(
                                "wait-quiescence",
                                id,
                                FixtureErrorKind::UnknownAttempt,
                            )
                        })?;
                    return Ok(record);
                }
                Some(false) => {}
                None => {
                    return Err(FixtureError::for_attempt(
                        "wait-quiescence",
                        id,
                        FixtureErrorKind::UnknownAttempt,
                    ));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::for_attempt(
                    "wait-quiescence",
                    id,
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, wait) =
                self.shared
                    .changed
                    .wait_timeout(state, remaining)
                    .map_err(|_| {
                        FixtureError::for_attempt("wait-quiescence", id, FixtureErrorKind::Poisoned)
                    })?;
            state = next;
            if wait.timed_out() {
                continue;
            }
        }
    }

    pub(super) fn finish(
        &self,
        owner_proof: TranslationOwnerQuiesced,
    ) -> Result<Vec<AttemptRecord>, FixtureError> {
        let deadline = Instant::now()
            .checked_add(FIXTURE_WATCHDOG)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| FixtureError::new("finish", FixtureErrorKind::Poisoned))?;
        if state.finished {
            return Err(FixtureError::new(
                "finish",
                FixtureErrorKind::AlreadyFinished,
            ));
        }
        if !state
            .owner
            .as_ref()
            .is_some_and(|owner| owner.accepts(&owner_proof))
        {
            return Err(FixtureError::new("finish", FixtureErrorKind::WrongOwner));
        }

        // The exact owner has joined, so closing this fixture prevents every
        // remaining detached attempt from entering a new adapter/delay phase.
        // Wait for the adapter object's final Drop as the acknowledgement that
        // no pre-Begin detached thread can appear after this drain.
        state.finished = true;
        self.shared.changed.notify_all();
        while !stopped_owner_lifetimes_drained(&state) {
            if state.adapter_alive {
                state.finish_waiting_for_adapter = true;
                self.shared.changed.notify_all();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "finish",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| FixtureError::new("finish", FixtureErrorKind::Poisoned))?;
            state = next;
        }
        drop(state);
        self.validate_quiescence()
    }

    fn validate_quiescence(&self) -> Result<Vec<AttemptRecord>, FixtureError> {
        let records = {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| FixtureError::new("finish", FixtureErrorKind::Poisoned))?;
            if let Some(error) = state.first_error {
                return Err(error);
            }
            if !state.scripts.is_empty() {
                return Err(FixtureError::new("finish", FixtureErrorKind::UnusedScripts));
            }
            if !all_attempts_fully_quiesced(&state) {
                return Err(FixtureError::new(
                    "finish",
                    FixtureErrorKind::AttemptsStillActive,
                ));
            }
            if state.delay_calls_inflight != 0 {
                return Err(FixtureError::new(
                    "finish",
                    FixtureErrorKind::DelaysStillActive,
                ));
            }
            state.records.clone()
        };
        let clock = self
            .shared
            .clock
            .state
            .lock()
            .map_err(|_| FixtureError::new("finish", FixtureErrorKind::Poisoned))?;
        if clock.active_delays != 0 || clock.delays.iter().any(|delay| delay.finished_at.is_none())
        {
            return Err(FixtureError::new(
                "finish",
                FixtureErrorKind::DelaysStillActive,
            ));
        }
        Ok(records)
    }

    pub(super) fn delay_records(&self) -> Result<Vec<DelayRecord>, FixtureError> {
        self.shared
            .clock
            .state
            .lock()
            .map(|state| state.delays.clone())
            .map_err(|_| FixtureError::new("delay-records", FixtureErrorKind::Poisoned))
    }

    fn quiescence_probe(&self, id: AttemptId) -> FixtureQuiescenceProbe {
        FixtureQuiescenceProbe {
            shared: Arc::clone(&self.shared),
            id,
        }
    }

    fn lifetime_probe(&self) -> FixtureLifetimeProbe {
        FixtureLifetimeProbe {
            shared: Arc::clone(&self.shared),
        }
    }

    fn pause_before_begin_guard(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pause_before_begin_guard = true;
    }

    fn wait_until_before_begin_guard_paused(&self, watchdog: Duration) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.before_begin_guard_paused {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "wait-before-begin-guard",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        Ok(())
    }

    fn release_before_begin_guard(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pause_before_begin_guard = false;
        self.shared.changed.notify_all();
    }

    fn wait_until_finish_waiting_for_adapter(
        &self,
        watchdog: Duration,
    ) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.finish_waiting_for_adapter {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "wait-finish-adapter",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        Ok(())
    }

    fn pause_after_begin_guard(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pause_after_begin_guard = true;
    }

    fn wait_until_begin_guard_paused(&self, watchdog: Duration) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.begin_guard_paused {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "wait-begin-guard",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        Ok(())
    }

    fn pause_after_attempt_publication(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pause_after_attempt_publication = true;
    }

    fn wait_until_attempt_publication_paused(
        &self,
        watchdog: Duration,
    ) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.attempt_publication_paused {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "wait-attempt-publication",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        Ok(())
    }
}

struct FixtureLifetimeProbe {
    shared: Arc<FixtureShared>,
}

impl FixtureLifetimeProbe {
    fn wait_until_drop_waiting_for_adapter(&self, watchdog: Duration) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.drop_waiting_for_adapter {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "probe-drop-adapter",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        Ok(())
    }

    fn wait(&self, watchdog: Duration) -> Result<(), FixtureError> {
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if all_fixture_lifetimes_quiesced(&state) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::new(
                    "probe-lifetime",
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
    }
}

struct FixtureQuiescenceProbe {
    shared: Arc<FixtureShared>,
    id: AttemptId,
}

impl FixtureQuiescenceProbe {
    fn wait(&self, watchdog: Duration) -> Result<(), FixtureError> {
        if !self.id.belongs_to(self.shared.identity) {
            return Err(FixtureError::for_attempt(
                "probe-quiescence",
                self.id,
                FixtureErrorKind::WrongOwner,
            ));
        }
        let deadline = Instant::now()
            .checked_add(watchdog)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let Some(quiesced) = attempt_is_fully_quiesced(&state, self.id) else {
                return Err(FixtureError::for_attempt(
                    "probe-quiescence",
                    self.id,
                    FixtureErrorKind::UnknownAttempt,
                ));
            };
            if quiesced {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FixtureError::for_attempt(
                    "probe-quiescence",
                    self.id,
                    FixtureErrorKind::WatchdogExpired,
                ));
            }
            let (next, _wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
    }
}

impl Drop for TranslationPolicyFixture {
    fn drop(&mut self) {
        let cleanup_time = self.shared.clock.now();
        let deadline = Instant::now()
            .checked_add(FIXTURE_WATCHDOG)
            .unwrap_or_else(Instant::now);
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.cleaning_up = true;
            self.shared.changed.notify_all();
        }
        {
            let mut clock = self
                .shared
                .clock
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            clock.cancelled = true;
            self.shared.clock.changed.notify_all();
        }

        // `cleaning_up` closes BeginInflightGuard::enter. Waiting for every
        // guard already admitted before the final sweep makes callback/blocker
        // publication and cleanup linearizable even if Begin was pre-empted at
        // its publication boundary.
        let begins_drained = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if !state.owner_starting && state.begin_inflight == 0 {
                    break true;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break false;
                }
                let (next, _wait) = self
                    .shared
                    .changed
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state = next;
            }
        };
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut locally_quiesced = Vec::new();
            for (id, attempt) in &mut state.attempts {
                match attempt.phase {
                    AttemptPhase::HeldPending { .. } => {
                        attempt.completion.take();
                        attempt.phase = AttemptPhase::Quiesced;
                        locally_quiesced.push((
                            *id,
                            Some(CancellationStatus::Confirmed),
                            Some(AttemptTerminal::CancelledConfirmed),
                        ));
                    }
                    AttemptPhase::CancelledUnconfirmed => {
                        attempt.completion.take();
                        attempt.phase = AttemptPhase::Quiesced;
                        locally_quiesced.push((*id, None, None));
                    }
                    AttemptPhase::NonCooperativeBlocked => {
                        attempt.phase = AttemptPhase::NonCooperativeReleaseRequested;
                    }
                    AttemptPhase::Resolving
                    | AttemptPhase::NonCooperativeReleaseRequested
                    | AttemptPhase::Quiesced => {}
                }
            }
            for (id, cancellation, terminal) in locally_quiesced {
                if let Some(record) = state.records.iter_mut().find(|record| record.id == id) {
                    record.finished_at.get_or_insert(cleanup_time);
                    if let Some(cancellation) = cancellation {
                        record.cancellation = Some(cancellation);
                    }
                    if let Some(terminal) = terminal {
                        record.terminal = Some(terminal);
                    }
                    record.quiesced = true;
                }
            }
            self.shared.changed.notify_all();
        }
        let attempts_quiesced = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                let quiesced = begins_drained && all_fixture_lifetimes_quiesced(&state);
                if quiesced {
                    break true;
                }
                if state.adapter_alive || state.adapter_calls_inflight != 0 {
                    state.drop_waiting_for_adapter = true;
                    self.shared.changed.notify_all();
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break false;
                }
                let (next, wait) = self
                    .shared
                    .changed
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state = next;
                if wait.timed_out() {
                    continue;
                }
            }
        };
        let delays_quiesced = {
            let mut clock = self
                .shared
                .clock
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                let quiesced = clock.active_delays == 0
                    && clock.delays.iter().all(|delay| delay.finished_at.is_some());
                if quiesced {
                    break true;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break false;
                }
                let (next, wait) = self
                    .shared
                    .clock
                    .changed
                    .wait_timeout(clock, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                clock = next;
                if wait.timed_out() {
                    continue;
                }
            }
        };
        if !attempts_quiesced || !delays_quiesced {
            if std::thread::panicking() {
                tracing::warn!(
                    attempts_quiesced,
                    delays_quiesced,
                    "Translation policy fixture cleanup exceeded its watchdog while unwinding"
                );
            } else {
                std::panic::resume_unwind(Box::new(
                    "Translation policy fixture cleanup exceeded its watchdog",
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "policy_test_fixture_tests.rs"]
mod tests;
