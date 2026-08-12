use super::*;
use crate::caption::{CaptionAggregateStore, CaptionLane, CaptionSnapshot, CaptionState};
use crate::config::{ApiBaseUrl, TranslationConfig, TranslationEndpoint, TranslationPath};
use crate::credentials::{CredentialId, CredentialStorage, ResolvedCredential};
use crate::error::{AppError, AppResult};
use crate::host_resolver::HostResolver;
use secrecy::SecretString;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct ScriptedAdapter {
    state: Arc<Mutex<ScriptedAdapterState>>,
}

struct ScriptedAdapterState {
    results: VecDeque<Result<String, AdapterFailure>>,
    sources: Vec<String>,
    targets: Vec<TranslationTarget>,
    budgets: Vec<(Duration, Duration)>,
}

impl ScriptedAdapter {
    fn successes<const N: usize>(translations: [&str; N]) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedAdapterState {
                results: translations
                    .into_iter()
                    .map(|translation| Ok(translation.to_string()))
                    .collect(),
                sources: Vec::new(),
                targets: Vec::new(),
                budgets: Vec::new(),
            })),
        }
    }

    fn sources(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|state| state.sources.clone())
            .unwrap_or_default()
    }

    fn targets(&self) -> Vec<TranslationTarget> {
        self.state
            .lock()
            .map(|state| state.targets.clone())
            .unwrap_or_default()
    }

    fn budgets(&self) -> Vec<(Duration, Duration)> {
        self.state
            .lock()
            .map(|state| state.budgets.clone())
            .unwrap_or_default()
    }

    fn with_results(results: impl IntoIterator<Item = Result<String, AdapterFailure>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedAdapterState {
                results: results.into_iter().collect(),
                sources: Vec::new(),
                targets: Vec::new(),
                budgets: Vec::new(),
            })),
        }
    }
}

impl CompletedTextAdapter for ScriptedAdapter {
    fn begin(
        &self,
        request: CompletedTextRequest,
        control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        let mut state = self.state.lock().map_err(|_| AdapterFailure {
            class: TranslationFailureClass::Unknown,
            retryable: false,
            retry_after: None,
            request_outcome_ambiguous: false,
        })?;
        state.sources.push(request.source_text);
        state.targets.push(request.target);
        state
            .budgets
            .push((control.attempt_budget, control.total_budget));
        let result = state.results.pop_front().unwrap_or(Err(AdapterFailure {
            class: TranslationFailureClass::Unknown,
            retryable: false,
            retry_after: None,
            request_outcome_ambiguous: false,
        }));
        drop(state);
        completion.finish(result);
        Ok(Box::new(NoopActiveCall))
    }
}

struct NoopActiveCall;

impl ActiveTranslationCall for NoopActiveCall {
    fn cancel(&mut self) -> CancellationStatus {
        CancellationStatus::Confirmed
    }
}

#[derive(Default)]
struct ManualClock {
    now_ms: std::sync::atomic::AtomicU64,
}

impl ManualClock {
    fn advance(&self, duration: Duration) {
        self.now_ms.fetch_add(
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }
}

impl TranslationClock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.now_ms.load(Ordering::SeqCst))
    }
}

struct AdvancingDelay {
    clock: Arc<ManualClock>,
    waits: Mutex<Vec<Duration>>,
}

impl AdvancingDelay {
    fn new(clock: Arc<ManualClock>) -> Self {
        Self {
            clock,
            waits: Mutex::new(Vec::new()),
        }
    }

    fn waits(&self) -> Vec<Duration> {
        self.waits
            .lock()
            .map(|waits| waits.clone())
            .unwrap_or_default()
    }
}

impl CancellableDelay for AdvancingDelay {
    fn wait(
        &self,
        duration: Duration,
        stopped: &AtomicBool,
        _clock: &dyn TranslationClock,
    ) -> bool {
        if stopped.load(Ordering::SeqCst) {
            return false;
        }
        if let Ok(mut waits) = self.waits.lock() {
            waits.push(duration);
        }
        self.clock.advance(duration);
        true
    }
}

#[derive(Default)]
struct StopAwareBlockingDelay {
    entered: AtomicBool,
}

impl StopAwareBlockingDelay {
    fn wait_until_entered(&self, timeout: Duration) -> AppResult<()> {
        let deadline = std::time::Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(std::time::Instant::now);
        while !self.entered.load(Ordering::SeqCst) {
            if std::time::Instant::now() >= deadline {
                return Err(AppError::state("Retry backoff did not begin in time."));
            }
            std::thread::yield_now();
        }
        Ok(())
    }
}

impl CancellableDelay for StopAwareBlockingDelay {
    fn wait(
        &self,
        _duration: Duration,
        stopped: &AtomicBool,
        _clock: &dyn TranslationClock,
    ) -> bool {
        self.entered.store(true, Ordering::SeqCst);
        while !stopped.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        false
    }
}

struct FixedJitter(Duration);

impl RetryJitter for FixedJitter {
    fn delay(&self, _base: Duration) -> Duration {
        self.0
    }
}

struct QueuedNearDeadlineAdapter {
    clock: Arc<ManualClock>,
    calls: std::sync::atomic::AtomicU64,
    budgets: Mutex<Vec<(Duration, Duration)>>,
}

#[derive(Clone)]
struct LateAmbiguousAdapter {
    clock: Arc<ManualClock>,
    calls: Arc<std::sync::atomic::AtomicU64>,
}

impl CompletedTextAdapter for LateAmbiguousAdapter {
    fn begin(
        &self,
        _request: CompletedTextRequest,
        _control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.clock
            .advance(ATTEMPT_DEADLINE + Duration::from_millis(1));
        completion.finish(Err(AdapterFailure {
            class: TranslationFailureClass::DeadlineExceeded,
            retryable: false,
            retry_after: None,
            request_outcome_ambiguous: true,
        }));
        Ok(Box::new(NoopActiveCall))
    }
}

impl QueuedNearDeadlineAdapter {
    fn new(clock: Arc<ManualClock>) -> Self {
        Self {
            clock,
            calls: std::sync::atomic::AtomicU64::new(0),
            budgets: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    fn budgets(&self) -> Vec<(Duration, Duration)> {
        self.budgets
            .lock()
            .map(|budgets| budgets.clone())
            .unwrap_or_default()
    }
}

impl CompletedTextAdapter for QueuedNearDeadlineAdapter {
    fn begin(
        &self,
        _request: CompletedTextRequest,
        control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        if let Ok(mut budgets) = self.budgets.lock() {
            budgets.push((control.attempt_budget, control.total_budget));
        }
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call <= 2 {
            self.clock.advance(Duration::from_millis(4_900));
            completion.finish(Ok(format!("translated-{call}")));
        } else {
            self.clock.advance(Duration::from_millis(1_900));
            completion.finish(Err(AdapterFailure {
                class: TranslationFailureClass::RateLimited,
                retryable: true,
                retry_after: Some(Duration::from_secs(30)),
                request_outcome_ambiguous: false,
            }));
        }
        Ok(Box::new(NoopActiveCall))
    }
}

#[derive(Clone)]
struct BlockingAdapter {
    shared: Arc<BlockingAdapterShared>,
}

#[derive(Default)]
struct BlockingAdapterShared {
    state: Mutex<BlockingAdapterState>,
    changed: Condvar,
}

#[derive(Default)]
struct BlockingAdapterState {
    calls: usize,
    cancelled: bool,
    completion: Option<AdapterCompletion>,
}

impl Default for BlockingAdapter {
    fn default() -> Self {
        Self {
            shared: Arc::new(BlockingAdapterShared::default()),
        }
    }
}

impl BlockingAdapter {
    fn wait_until_called(&self, timeout: Duration) -> AppResult<()> {
        self.wait_until_call_count(1, timeout)
    }

    fn wait_until_call_count(&self, expected: usize, timeout: Duration) -> AppResult<()> {
        let deadline = std::time::Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(std::time::Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| AppError::state("Blocking Adapter test lock was poisoned."))?;
        while state.calls < expected {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(AppError::state("Blocking Adapter was not called in time."));
            }
            let (next_state, wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| AppError::state("Blocking Adapter test wait was poisoned."))?;
            state = next_state;
            if wait.timed_out() && state.calls < expected {
                return Err(AppError::state("Blocking Adapter was not called in time."));
            }
        }
        Ok(())
    }

    fn was_cancelled(&self) -> bool {
        self.shared
            .state
            .lock()
            .map(|state| state.cancelled)
            .unwrap_or_default()
    }

    fn complete(&self, text: &str) {
        let completion = self
            .shared
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.completion.take());
        if let Some(completion) = completion {
            completion.finish(Ok(text.to_string()));
        }
    }

    fn calls(&self) -> usize {
        self.shared
            .state
            .lock()
            .map(|state| state.calls)
            .unwrap_or_default()
    }
}

impl CompletedTextAdapter for BlockingAdapter {
    fn begin(
        &self,
        _request: CompletedTextRequest,
        _control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        let mut state = self.shared.state.lock().map_err(|_| AdapterFailure {
            class: TranslationFailureClass::Unknown,
            retryable: false,
            retry_after: None,
            request_outcome_ambiguous: false,
        })?;
        state.calls = state.calls.saturating_add(1);
        state.completion = Some(completion);
        drop(state);
        self.shared.changed.notify_all();
        Ok(Box::new(BlockingActiveCall {
            shared: Arc::clone(&self.shared),
        }))
    }
}

struct BlockingActiveCall {
    shared: Arc<BlockingAdapterShared>,
}

impl ActiveTranslationCall for BlockingActiveCall {
    fn cancel(&mut self) -> CancellationStatus {
        if let Ok(mut state) = self.shared.state.lock() {
            state.cancelled = true;
            // Dropping the only callback sender makes this scripted call
            // incapable of completing after cancellation returns.
            state.completion.take();
            return CancellationStatus::Confirmed;
        }
        CancellationStatus::Unconfirmed
    }
}

#[derive(Clone, Default)]
struct NonCooperativeAdapter {
    release: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct UnconfirmedCancelAdapter {
    shared: Arc<UnconfirmedCancelShared>,
}

#[derive(Default)]
struct UnconfirmedCancelShared {
    calls: std::sync::atomic::AtomicU64,
    begun: Mutex<bool>,
    changed: Condvar,
    release: AtomicBool,
    completion: Mutex<Option<AdapterCompletion>>,
}

impl UnconfirmedCancelAdapter {
    fn wait_until_called(&self, timeout: Duration) -> AppResult<()> {
        let deadline = std::time::Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(std::time::Instant::now);
        let mut begun = self
            .shared
            .begun
            .lock()
            .map_err(|_| AppError::state("Unconfirmed Adapter test lock was poisoned."))?;
        while !*begun {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(AppError::state(
                    "Unconfirmed Adapter was not called in time.",
                ));
            }
            let (next, wait) = self
                .shared
                .changed
                .wait_timeout(begun, remaining)
                .map_err(|_| AppError::state("Unconfirmed Adapter test wait was poisoned."))?;
            begun = next;
            if wait.timed_out() && !*begun {
                return Err(AppError::state(
                    "Unconfirmed Adapter was not called in time.",
                ));
            }
        }
        Ok(())
    }

    fn calls(&self) -> u64 {
        self.shared.calls.load(Ordering::SeqCst)
    }

    fn release(&self) {
        self.shared.release.store(true, Ordering::SeqCst);
    }
}

impl CompletedTextAdapter for UnconfirmedCancelAdapter {
    fn begin(
        &self,
        _request: CompletedTextRequest,
        _control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        self.shared.calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut begun) = self.shared.begun.lock() {
            *begun = true;
        }
        self.shared.changed.notify_all();
        if let Ok(mut stored) = self.shared.completion.lock() {
            *stored = Some(completion);
        }
        Ok(Box::new(UnconfirmedActiveCall {
            shared: Arc::clone(&self.shared),
        }))
    }
}

struct UnconfirmedActiveCall {
    shared: Arc<UnconfirmedCancelShared>,
}

impl ActiveTranslationCall for UnconfirmedActiveCall {
    fn cancel(&mut self) -> CancellationStatus {
        while !self.shared.release.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        CancellationStatus::Unconfirmed
    }
}

impl NonCooperativeAdapter {
    fn release(&self) {
        self.release.store(true, Ordering::SeqCst);
    }
}

impl CompletedTextAdapter for NonCooperativeAdapter {
    fn begin(
        &self,
        _request: CompletedTextRequest,
        _control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        while !self.release.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        completion.finish(Ok("late private translation".to_string()));
        Ok(Box::new(NoopActiveCall))
    }
}

fn reservation(
    store: &CaptionAggregateStore,
    generation: u64,
    unit: &str,
    text: impl Into<String>,
) -> AppResult<crate::caption::ReservedCompletedSource> {
    let active = store
        .begin_generation(generation)?
        .active_stream
        .ok_or_else(|| {
            AppError::state("Test generation did not produce an active caption stream.")
        })?;
    store.start_unit(generation, &active.stream_id, unit.to_string(), 10)?;
    let caption = CaptionSnapshot {
        generation,
        stream_id: active.stream_id,
        unit_id: Some(unit.to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: text.into(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(10),
        timestamp_ms: 20,
    };
    store
        .accept_completed_source_for_translation(caption)?
        .map(|(_, reservation)| reservation)
        .ok_or_else(|| AppError::state("Test source was not reserved."))
}

fn accept_completed_source(
    store: &CaptionAggregateStore,
    generation: u64,
    unit: &str,
    text: impl Into<String>,
) -> AppResult<CaptionSnapshot> {
    let active = store
        .begin_generation(generation)?
        .active_stream
        .ok_or_else(|| AppError::state("Test generation did not produce an active stream."))?;
    store.start_unit(generation, &active.stream_id, unit.to_string(), 10)?;
    let caption = CaptionSnapshot {
        generation,
        stream_id: active.stream_id,
        unit_id: Some(unit.to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: text.into(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(10),
        timestamp_ms: 20,
    };
    store.accept_caption(caption.clone())?;
    Ok(caption)
}

fn translation_credential(id: CredentialId) -> ResolvedCredential {
    ResolvedCredential {
        id,
        secret: SecretString::from("test-translation-secret"),
        storage: CredentialStorage::SystemCredentialStore,
        display_suffix: Some("test".to_string()),
    }
}

#[test]
fn production_factory_binds_each_endpoint_to_its_own_credential() -> AppResult<()> {
    let custom_base =
        ApiBaseUrl::parse("https://translation.example.test/api/v1").map_err(AppError::config)?;
    let cases = [
        (
            TranslationEndpoint::Official,
            CredentialId::OpenAi,
            CredentialId::CustomTranslation,
        ),
        (
            TranslationEndpoint::Custom {
                api_base_url: custom_base,
            },
            CredentialId::CustomTranslation,
            CredentialId::OpenAi,
        ),
    ];

    for (endpoint, expected, mismatched) in cases {
        let selection = TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::English,
            endpoint,
        };
        let binding = openai_responses_completed_text_module(
            selection.clone(),
            translation_credential(expected),
            1,
            HostResolver::default(),
        )?;
        binding.stop_for_test()?;

        let error = openai_responses_completed_text_module(
            selection,
            translation_credential(mismatched),
            1,
            HostResolver::default(),
        )
        .err()
        .ok_or_else(|| AppError::state("A mismatched Translation credential was accepted."))?;
        assert_eq!(error.code(), "runtime.failed");
    }
    Ok(())
}

#[test]
fn completed_work_is_fifo_and_correlation_stays_owned_by_the_module() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::successes(["one translated", "two translated"]);
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter.clone()),
        TestPolicyDependencies::real(),
    )?;
    assert!(matches!(
        outcomes.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));

    module
        .try_submit(reservation(&store, 1, "one", "first private source")?)
        .map_err(|_| AppError::state("First source was rejected."))?;
    module
        .try_submit(reservation(&store, 1, "two", "second private source")?)
        .map_err(|_| AppError::state("Second source was rejected."))?;

    let first = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("First terminal translation outcome was not received."))?;
    let second = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Second terminal translation outcome was not received."))?;
    assert_eq!(first.source_ref().unit_id, "one");
    assert_eq!(second.source_ref().unit_id, "two");
    assert!(matches!(first, TranslationTerminalOutcome::Completed(_)));
    assert!(matches!(second, TranslationTerminalOutcome::Completed(_)));
    assert_eq!(
        adapter.sources(),
        ["first private source", "second private source"]
    );
    let TranslationTerminalOutcome::Completed(first) = first else {
        return Err(AppError::state("First translation was not completed."));
    };
    let aggregate_update = first
        .complete(30)?
        .ok_or_else(|| AppError::state("Completed translation was not finalized."))?;
    assert!(matches!(
        aggregate_update.change,
        crate::caption::CaptionAggregateChange::CaptionAccepted(CaptionSnapshot {
            lane: CaptionLane::Translation,
            ..
        })
    ));

    module.stop()?;
    Ok(())
}

#[test]
fn admission_enforces_count_and_source_byte_budgets_without_waiting() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let (mut module, _outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        adapter,
        TestPolicyDependencies::real(),
    )?;

    for index in 0_u64..8 {
        assert_eq!(
            module
                .try_submit(reservation(
                    &store,
                    2,
                    &format!("unit-{index}"),
                    "x".repeat(8 * 1024),
                )?)
                .map_err(|rejection| rejection.kind()),
            Ok(())
        );
    }
    assert_eq!(
        module
            .try_submit(reservation(&store, 2, "ninth", "x")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::OutstandingLimit)
    );
    assert_eq!(
        module
            .try_submit(reservation(
                &store,
                2,
                "oversized",
                "x".repeat(16 * 1024 + 1),
            )?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::SourceTooLarge)
    );

    module.stop()?;
    Ok(())
}

#[test]
fn rejected_submission_retains_the_one_shot_failure_capability() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let (mut module, _outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        adapter,
        TestPolicyDependencies::real(),
    )?;
    for index in 0_u64..OUTSTANDING_LIMIT as u64 {
        module
            .try_submit(reservation(
                &store,
                31,
                &format!("admitted-{index}"),
                "admitted source",
            )?)
            .map_err(|_| AppError::state("Budget setup source was rejected."))?;
    }

    let rejection = module
        .try_submit(reservation(&store, 31, "rejected", "rejected source")?)
        .err()
        .ok_or_else(|| AppError::state("Over-budget source was unexpectedly admitted."))?;
    assert_eq!(rejection.kind(), TranslationSubmitError::OutstandingLimit);
    assert_eq!(
        rejection.reason(),
        crate::caption::TranslationFailureReason::Backpressure
    );
    assert!(!format!("{rejection:?}").contains("rejected source"));
    assert!(store.snapshot()?.translation_units.iter().any(|unit| {
        matches!(
            unit,
            crate::caption::TranslationUnitSnapshot::Pending { source_ref }
                if source_ref.unit_id == "rejected"
        )
    }));

    let update = rejection
        .fail()?
        .ok_or_else(|| AppError::state("Rejected Translation was not terminalized."))?;
    assert!(matches!(
        update.change,
        crate::caption::CaptionAggregateChange::TranslationFailed(
            crate::caption::TranslationUnitSnapshot::Failed {
                reason_code: crate::caption::TranslationFailureReason::Backpressure,
                ..
            }
        )
    ));
    module.stop()?;
    Ok(())
}

#[test]
fn completed_outcomes_hold_source_bytes_until_finalized_or_dropped() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::successes(["one", "two", "three", "four", "five"]);
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter),
        TestPolicyDependencies::real(),
    )?;
    for index in 0_u64..4 {
        module
            .try_submit(reservation(
                &store,
                13,
                &format!("held-{index}"),
                "x".repeat(SOURCE_BYTE_LIMIT),
            )?)
            .map_err(|_| AppError::state("Held-outcome test source was rejected."))?;
    }
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(
            outcomes
                .recv_timeout(TEST_TIMEOUT)
                .map_err(|_| AppError::state("Held completed outcome was not received."))?,
        );
    }

    assert_eq!(
        module
            .try_submit(reservation(
                &store,
                13,
                "retained-limit",
                "x".repeat(SOURCE_BYTE_LIMIT),
            )?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::RetainedSourceLimit)
    );
    drop(held.pop());
    assert_eq!(
        module
            .try_submit(reservation(
                &store,
                13,
                "after-release",
                "x".repeat(SOURCE_BYTE_LIMIT),
            )?)
            .map_err(|rejection| rejection.kind()),
        Ok(())
    );
    module.stop()?;
    Ok(())
}

#[test]
fn unconsumed_outcomes_continue_to_hold_the_outstanding_slots() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::successes([
        "one", "two", "three", "four", "five", "six", "seven", "eight",
    ]);
    let observed_adapter = adapter.clone();
    let (mut module, _outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter),
        TestPolicyDependencies::real(),
    )?;
    for index in 0_u64..8 {
        module
            .try_submit(reservation(&store, 14, &format!("outcome-{index}"), "x")?)
            .map_err(|_| AppError::state("Unconsumed-outcome test source was rejected."))?;
    }
    let deadline = std::time::Instant::now()
        .checked_add(TEST_TIMEOUT)
        .unwrap_or_else(std::time::Instant::now);
    while observed_adapter.sources().len() < 8 {
        if std::time::Instant::now() >= deadline {
            return Err(AppError::state(
                "Unconsumed outcomes were not produced in time.",
            ));
        }
        std::thread::yield_now();
    }
    assert_eq!(
        module
            .try_submit(reservation(&store, 14, "ninth", "x")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::OutstandingLimit)
    );
    module.stop()?;
    Ok(())
}

#[test]
fn stop_releases_active_and_queued_reservations_and_suppresses_late_results() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        TestPolicyDependencies::real(),
    )?;
    module
        .try_submit(reservation(&store, 3, "active", "active private source")?)
        .map_err(|_| AppError::state("Active test source was rejected."))?;
    module
        .try_submit(reservation(&store, 3, "queued", "queued private source")?)
        .map_err(|_| AppError::state("Queued test source was rejected."))?;
    adapter.wait_until_called(TEST_TIMEOUT)?;

    module.stop()?;
    adapter.complete("late private translation");
    assert!(outcomes.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(
        module
            .try_submit(reservation(&store, 3, "late", "late source")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Stopped)
    );

    Ok(())
}

#[test]
fn stop_does_not_wait_for_a_non_cooperative_adapter_to_return() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(NonCooperativeAdapter::default());
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        TestPolicyDependencies::real(),
    )?;
    module
        .try_submit(reservation(&store, 4, "non-cooperative", "private source")?)
        .map_err(|_| AppError::state("Non-cooperative test source was rejected."))?;

    let started = std::time::Instant::now();
    module.stop()?;
    assert!(started.elapsed() < Duration::from_millis(100));
    adapter.release();
    assert!(outcomes.recv_timeout(Duration::from_millis(50)).is_err());

    Ok(())
}

#[test]
fn retryable_failure_uses_two_attempts_and_the_deterministic_backoff() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::with_results([
        Err(AdapterFailure {
            class: TranslationFailureClass::ServiceUnavailable,
            retryable: true,
            retry_after: None,
            request_outcome_ambiguous: false,
        }),
        Ok("translated".to_string()),
    ]);
    let clock = Arc::new(ManualClock::default());
    let delay = Arc::new(AdvancingDelay::new(Arc::clone(&clock)));
    let dependencies = TestPolicyDependencies {
        clock,
        delay: delay.clone(),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter.clone()),
        dependencies,
    )?;
    module
        .try_submit(reservation(&store, 5, "retry", "private source")?)
        .map_err(|_| AppError::state("Retry test source was rejected."))?;

    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    assert_eq!(adapter.sources().len(), 2);
    assert_eq!(
        adapter.targets(),
        [
            TranslationTarget::SimplifiedChinese,
            TranslationTarget::SimplifiedChinese,
        ]
    );
    assert_eq!(
        adapter.budgets(),
        [
            (Duration::from_secs(5), Duration::from_secs(12)),
            (Duration::from_secs(5), Duration::from_millis(11_750)),
        ]
    );
    assert_eq!(delay.waits(), [Duration::from_millis(250)]);
    module.stop()?;
    Ok(())
}

#[test]
fn confirmed_attempt_timeout_cancels_before_retrying() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let clock = Arc::new(ManualClock::default());
    let delay = Arc::new(AdvancingDelay::new(Arc::clone(&clock)));
    let dependencies = TestPolicyDependencies {
        clock: clock.clone(),
        delay: delay.clone(),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        dependencies,
    )?;
    module
        .try_submit(reservation(
            &store,
            24,
            "attempt-deadline",
            "private source",
        )?)
        .map_err(|_| AppError::state("Attempt-deadline test source was rejected."))?;
    adapter.wait_until_called(TEST_TIMEOUT)?;

    clock.advance(ATTEMPT_DEADLINE);
    adapter.wait_until_call_count(2, TEST_TIMEOUT)?;
    assert!(adapter.was_cancelled());
    assert_eq!(delay.waits(), [Duration::from_millis(250)]);
    adapter.complete("translated");
    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    assert_eq!(adapter.calls(), 2);
    module.stop()?;
    Ok(())
}

#[test]
fn stop_cancels_retry_backoff_before_another_call_starts() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::with_results([
        Err(AdapterFailure {
            class: TranslationFailureClass::ServiceUnavailable,
            retryable: true,
            retry_after: None,
            request_outcome_ambiguous: false,
        }),
        Ok("must not run".to_string()),
    ]);
    let observed_adapter = adapter.clone();
    let delay = Arc::new(StopAwareBlockingDelay::default());
    let dependencies = TestPolicyDependencies {
        clock: Arc::new(ManualClock::default()),
        delay: delay.clone(),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter),
        dependencies,
    )?;
    module
        .try_submit(reservation(&store, 25, "stop-backoff", "private source")?)
        .map_err(|_| AppError::state("Stop-backoff test source was rejected."))?;
    delay.wait_until_entered(TEST_TIMEOUT)?;

    let started = std::time::Instant::now();
    module.stop()?;
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(observed_adapter.sources().len(), 1);
    assert!(outcomes.recv_timeout(Duration::from_millis(50)).is_err());
    Ok(())
}

#[test]
fn terminal_provider_failures_preserve_the_closed_provider_neutral_class() -> AppResult<()> {
    for (generation, class) in [
        (20, TranslationFailureClass::Authentication),
        (21, TranslationFailureClass::PermissionDenied),
        (22, TranslationFailureClass::InvalidRequest),
        (23, TranslationFailureClass::UsageLimit),
    ] {
        let store = CaptionAggregateStore::default();
        let adapter = ScriptedAdapter::with_results([Err(AdapterFailure {
            class,
            retryable: false,
            retry_after: None,
            request_outcome_ambiguous: false,
        })]);
        let (mut module, outcomes) = TranslationModule::start_for_test(
            TranslationTarget::SimplifiedChinese,
            Arc::new(adapter),
            TestPolicyDependencies::real(),
        )?;
        module
            .try_submit(reservation(
                &store,
                generation,
                "provider-failure",
                "private source",
            )?)
            .map_err(|_| AppError::state("Provider-failure test source was rejected."))?;

        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Provider failure was not received."))?;
        let TranslationTerminalOutcome::Failed(failed) = outcome else {
            return Err(AppError::state(
                "Provider failure unexpectedly completed translation.",
            ));
        };
        assert_eq!(failed.class, class);
        module.stop()?;
    }
    Ok(())
}

#[test]
fn failed_outcome_keeps_its_source_reserved_until_failure_is_recorded() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::with_results([Err(AdapterFailure {
        class: TranslationFailureClass::ServiceUnavailable,
        retryable: false,
        retry_after: None,
        request_outcome_ambiguous: false,
    })]);
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter),
        TestPolicyDependencies::real(),
    )?;
    module
        .try_submit(reservation(
            &store,
            30,
            "failed",
            "source retained through failure",
        )?)
        .map_err(|_| AppError::state("Failure-retention source was rejected."))?;

    let outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Failure-retention outcome was not received."))?;
    let TranslationTerminalOutcome::Failed(failed) = outcome else {
        return Err(AppError::state(
            "Failure-retention work unexpectedly completed.",
        ));
    };
    for index in 0_u64..6 {
        accept_completed_source(
            &store,
            30,
            &format!("pressure-{index}"),
            format!("pressure source {index}"),
        )?;
    }
    assert!(
        store
            .snapshot()?
            .captions
            .iter()
            .any(|caption| caption.unit_id.as_deref() == Some("failed"))
    );

    let update = failed
        .fail()?
        .ok_or_else(|| AppError::state("Translation failure was not recorded."))?;
    assert!(matches!(
        update.change,
        crate::caption::CaptionAggregateChange::TranslationFailed(
            crate::caption::TranslationUnitSnapshot::Failed {
                reason_code: crate::caption::TranslationFailureReason::ProviderUnavailable,
                ..
            }
        )
    ));
    module.stop()?;
    Ok(())
}

#[test]
fn provider_neutral_failure_classes_map_to_closed_aggregate_reasons() {
    let cases = [
        (
            TranslationFailureClass::Authentication,
            crate::caption::TranslationFailureReason::ProviderAuthenticationFailed,
        ),
        (
            TranslationFailureClass::PermissionDenied,
            crate::caption::TranslationFailureReason::ProviderPermissionDenied,
        ),
        (
            TranslationFailureClass::InvalidRequest,
            crate::caption::TranslationFailureReason::ProviderInvalidRequest,
        ),
        (
            TranslationFailureClass::RateLimited,
            crate::caption::TranslationFailureReason::ProviderRateLimited,
        ),
        (
            TranslationFailureClass::UsageLimit,
            crate::caption::TranslationFailureReason::ProviderUsageLimit,
        ),
        (
            TranslationFailureClass::ServiceUnavailable,
            crate::caption::TranslationFailureReason::ProviderUnavailable,
        ),
        (
            TranslationFailureClass::InvalidOutput,
            crate::caption::TranslationFailureReason::InvalidOutput,
        ),
        (
            TranslationFailureClass::DeadlineExceeded,
            crate::caption::TranslationFailureReason::DeadlineExceeded,
        ),
        (
            TranslationFailureClass::Unknown,
            crate::caption::TranslationFailureReason::Failed,
        ),
    ];

    for (class, expected_reason) in cases {
        assert_eq!(class.reason(), expected_reason);
    }
}

#[test]
fn retry_after_beyond_the_total_budget_finishes_without_an_early_retry() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::with_results([
        Err(AdapterFailure {
            class: TranslationFailureClass::RateLimited,
            retryable: true,
            retry_after: Some(Duration::from_secs(30)),
            request_outcome_ambiguous: false,
        }),
        Ok("translated".to_string()),
    ]);
    let clock = Arc::new(ManualClock::default());
    let delay = Arc::new(AdvancingDelay::new(Arc::clone(&clock)));
    let dependencies = TestPolicyDependencies {
        clock,
        delay: delay.clone(),
        jitter: Arc::new(FixedJitter(Duration::from_millis(300))),
    };
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter.clone()),
        dependencies,
    )?;
    module
        .try_submit(reservation(&store, 6, "retry-after", "private source")?)
        .map_err(|_| AppError::state("Retry-After test source was rejected."))?;

    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Failed(FailedTranslation {
            class: TranslationFailureClass::RateLimited,
            ..
        }))
    ));
    assert_eq!(adapter.sources().len(), 1);
    assert!(delay.waits().is_empty());
    module.stop()?;
    Ok(())
}

#[test]
fn retry_after_within_the_total_budget_is_honored_without_jitter() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::with_results([
        Err(AdapterFailure {
            class: TranslationFailureClass::RateLimited,
            retryable: true,
            retry_after: Some(Duration::from_secs(2)),
            request_outcome_ambiguous: false,
        }),
        Ok("translated".to_string()),
    ]);
    let clock = Arc::new(ManualClock::default());
    let delay = Arc::new(AdvancingDelay::new(Arc::clone(&clock)));
    let dependencies = TestPolicyDependencies {
        clock,
        delay: delay.clone(),
        jitter: Arc::new(FixedJitter(Duration::from_millis(300))),
    };
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter.clone()),
        dependencies,
    )?;
    module
        .try_submit(reservation(
            &store,
            26,
            "retry-after-within-budget",
            "private source",
        )?)
        .map_err(|_| AppError::state("Retry-After test source was rejected."))?;

    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    assert_eq!(adapter.sources().len(), 2);
    assert_eq!(delay.waits(), [Duration::from_secs(2)]);
    module.stop()?;
    Ok(())
}

#[test]
fn queued_work_does_not_shorten_retry_after_to_fit_the_remaining_budget() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let clock = Arc::new(ManualClock::default());
    let adapter = Arc::new(QueuedNearDeadlineAdapter::new(Arc::clone(&clock)));
    let delay = Arc::new(AdvancingDelay::new(Arc::clone(&clock)));
    let dependencies = TestPolicyDependencies {
        clock,
        delay: delay.clone(),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        dependencies,
    )?;
    for unit in ["queue-one", "queue-two", "remaining-budget"] {
        module
            .try_submit(reservation(&store, 12, unit, "private source")?)
            .map_err(|_| AppError::state("Remaining-budget test source was rejected."))?;
    }

    for _ in 0..2 {
        assert!(matches!(
            outcomes.recv_timeout(TEST_TIMEOUT),
            Ok(TranslationTerminalOutcome::Completed(_))
        ));
    }

    let outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Remaining-budget failure was not received."))?;
    assert!(matches!(
        outcome,
        TranslationTerminalOutcome::Failed(FailedTranslation {
            class: TranslationFailureClass::RateLimited,
            ..
        })
    ));
    assert_eq!(adapter.calls(), 3);
    let budgets = adapter.budgets();
    assert_eq!(budgets.len(), 3);
    assert_eq!(
        budgets[0],
        (Duration::from_secs(5), Duration::from_secs(12))
    );
    assert!(budgets[1].0 <= Duration::from_secs(5));
    assert!(budgets[1].1 <= Duration::from_millis(7_100));
    assert_eq!(budgets[2].0, budgets[2].1);
    assert!(budgets[2].1 <= Duration::from_millis(2_200));
    assert!(delay.waits().is_empty());
    module.stop()?;
    Ok(())
}

#[test]
fn unconfirmed_attempt_timeout_fails_closed_without_starting_more_work() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(UnconfirmedCancelAdapter::default());
    let clock = Arc::new(ManualClock::default());
    let dependencies = TestPolicyDependencies {
        clock: clock.clone(),
        delay: Arc::new(AdvancingDelay::new(Arc::clone(&clock))),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        dependencies,
    )?;
    module
        .try_submit(reservation(
            &store,
            10,
            "attempt-timeout",
            "private source",
        )?)
        .map_err(|_| AppError::state("Attempt-timeout test source was rejected."))?;
    module
        .try_submit(reservation(&store, 10, "queued", "queued private source")?)
        .map_err(|_| AppError::state("Queued timeout test source was rejected."))?;
    adapter.wait_until_called(TEST_TIMEOUT)?;
    clock.advance(TOTAL_DEADLINE);

    for _ in 0..2 {
        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Attempt-timeout failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::DeadlineExceeded,
                ..
            })
        ));
    }
    assert_eq!(adapter.calls(), 1);
    assert_eq!(
        module
            .try_submit(reservation(&store, 10, "after-close", "private source")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Closed)
    );
    module.stop()?;
    adapter.release();
    Ok(())
}

#[test]
fn late_ambiguous_result_keeps_its_fail_closed_semantics() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let clock = Arc::new(ManualClock::default());
    let calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let adapter = LateAmbiguousAdapter {
        clock: Arc::clone(&clock),
        calls: Arc::clone(&calls),
    };
    let dependencies = TestPolicyDependencies {
        clock,
        delay: Arc::new(SystemDelay),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter),
        dependencies,
    )?;
    module
        .try_submit(reservation(&store, 29, "late-ambiguous", "private source")?)
        .map_err(|_| AppError::state("Late ambiguous source was rejected."))?;
    module
        .try_submit(reservation(
            &store,
            29,
            "must-not-start",
            "queued private source",
        )?)
        .map_err(|_| AppError::state("Queued ambiguous source was rejected."))?;

    for _ in 0..2 {
        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Ambiguous failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::DeadlineExceeded,
                ..
            })
        ));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        module
            .try_submit(reservation(
                &store,
                29,
                "closed-admission",
                "later private source",
            )?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Closed)
    ));
    module.stop()?;
    Ok(())
}

#[test]
fn total_deadline_starts_at_admission_and_includes_fifo_queue_time() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let clock = Arc::new(ManualClock::default());
    let dependencies = TestPolicyDependencies {
        clock: clock.clone(),
        delay: Arc::new(AdvancingDelay::new(Arc::clone(&clock))),
        jitter: Arc::new(FixedJitter(Duration::from_millis(250))),
    };
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        dependencies,
    )?;
    module
        .try_submit(reservation(&store, 11, "active", "first private source")?)
        .map_err(|_| AppError::state("Active deadline test source was rejected."))?;
    module
        .try_submit(reservation(&store, 11, "queued", "second private source")?)
        .map_err(|_| AppError::state("Queued deadline test source was rejected."))?;
    adapter.wait_until_called(TEST_TIMEOUT)?;
    clock.advance(TOTAL_DEADLINE);

    for _ in 0..2 {
        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Deadline failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::DeadlineExceeded,
                ..
            })
        ));
    }
    assert_eq!(adapter.calls(), 1);
    module.stop()?;
    Ok(())
}

#[test]
fn empty_and_oversized_outputs_are_safe_terminal_failures() -> AppResult<()> {
    for (generation, output) in [
        (7, "   ".to_string()),
        (8, "x".repeat(TRANSLATION_BYTE_LIMIT + 1)),
    ] {
        let store = CaptionAggregateStore::default();
        let adapter = ScriptedAdapter::with_results([Ok(output)]);
        let (mut module, outcomes) = TranslationModule::start_for_test(
            TranslationTarget::SimplifiedChinese,
            Arc::new(adapter),
            TestPolicyDependencies::real(),
        )?;
        module
            .try_submit(reservation(
                &store,
                generation,
                "invalid",
                "private source",
            )?)
            .map_err(|_| AppError::state("Invalid-output test source was rejected."))?;

        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Invalid-output failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::InvalidOutput,
                ..
            })
        ));
        module.stop()?;
    }
    Ok(())
}

#[test]
fn terminal_debug_does_not_expose_source_translation_or_provider_body() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = ScriptedAdapter::successes(["private translation"]);
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        Arc::new(adapter),
        TestPolicyDependencies::real(),
    )?;
    module
        .try_submit(reservation(&store, 9, "debug", "private source")?)
        .map_err(|_| AppError::state("Debug test source was rejected."))?;

    let outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Debug test outcome was not received."))?;
    let debug = format!("{outcome:?}");
    assert!(!debug.contains("private source"));
    assert!(!debug.contains("private translation"));
    module.stop()?;
    Ok(())
}

#[test]
fn production_retry_jitter_stays_within_twenty_percent() {
    let jitter = BoundedRetryJitter::new();
    for _ in 0..128 {
        let delay = jitter.delay(RETRY_BASE_DELAY);
        assert!(delay >= Duration::from_millis(200));
        assert!(delay <= Duration::from_millis(300));
    }
}

#[test]
fn dropping_outcome_receiver_stops_admission_and_releases_work() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let module_adapter: Arc<dyn CompletedTextAdapter> = adapter.clone();
    let (mut module, outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        module_adapter,
        TestPolicyDependencies::real(),
    )?;
    module
        .try_submit(reservation(&store, 15, "receiver-drop", "private source")?)
        .map_err(|_| AppError::state("Receiver-drop test source was rejected."))?;
    adapter.wait_until_called(TEST_TIMEOUT)?;

    drop(outcomes);
    assert_eq!(
        module
            .try_submit(reservation(&store, 15, "late", "late source")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Stopped)
    );
    module.stop()?;
    Ok(())
}

#[test]
fn stop_wins_an_admission_race_after_submission_preparation() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let adapter = Arc::new(BlockingAdapter::default());
    let (mut module, _outcomes) = TranslationModule::start_for_test(
        TranslationTarget::SimplifiedChinese,
        adapter,
        TestPolicyDependencies::real(),
    )?;
    let prepared = reservation(&store, 16, "race", "private source")?;
    let shared = Arc::clone(&module.shared);
    let result = module.try_submit_with_hook(prepared, move || shared.request_stop());

    assert_eq!(
        result.map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Stopped)
    );
    module.stop()?;
    Ok(())
}
