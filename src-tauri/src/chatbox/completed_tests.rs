use super::super::layout::PreparedChatboxText;
use super::super::text_pacing::Clock;
use super::super::transport::ChatboxSendReceipt;
use super::*;
use crate::generation_fence::{GenerationCommitter, GenerationFence};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::{Condvar, mpsc};

fn open_committer() -> GenerationCommitter {
    GenerationFence::new().committer()
}

fn close_at_fence(fence: &GenerationFence, publisher: &CompletedChatboxPublisher) -> AppResult<()> {
    fence.close_admission();
    let close_result = publisher.request_close(PublisherCloseReason::Stop);
    let commit_result = fence.wait_for_commits();
    match (close_result, commit_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(close_error), Err(commit_error)) => Err(AppError::state(format!(
            "Publisher and generation fence could not close: {close_error} {commit_error}"
        ))),
    }
}

fn wait_for_commits_closed(committer: &GenerationCommitter) -> AppResult<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !committer.is_closed() {
        if Instant::now() >= deadline {
            return Err(AppError::runtime("Stop did not close generation commits."));
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TransportEvent {
    Text(String),
    Typing(bool),
}

/// Waits until the recorded transport events satisfy `satisfied`. The
/// one-second bound is a deadlock diagnostic; every caller names a causal
/// milestone, such as the final typing-off after the expected pages.
fn wait_for_recorded<T: Clone>(
    events: &Mutex<Vec<T>>,
    changed: &Condvar,
    expectation: &str,
    satisfied: impl Fn(&[T]) -> bool,
) -> AppResult<Vec<T>> {
    let events = events
        .lock()
        .map_err(|_| AppError::state("Transport recording lock was poisoned."))?;
    let (events, timeout) = changed
        .wait_timeout_while(events, Duration::from_secs(1), |events| !satisfied(events))
        .map_err(|_| AppError::state("Transport recording lock was poisoned."))?;
    if timeout.timed_out() && !satisfied(&events) {
        return Err(AppError::runtime(format!(
            "Expected {expectation} within one second; recorded {} transport event(s).",
            events.len()
        )));
    }

    Ok(events.clone())
}

/// The publication milestone for a queue that drains completely: exactly
/// `text_count` text attempts, followed by the final typing-off transition.
fn texts_then_typing_off(events: &[TransportEvent], text_count: usize) -> bool {
    events
        .iter()
        .filter(|event| matches!(event, TransportEvent::Text(_)))
        .count()
        == text_count
        && events.last() == Some(&TransportEvent::Typing(false))
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

/// A policy clock for scripted sustained input. Worker sleeps that end by the
/// test-owned horizon complete immediately in policy time; a sleep that would
/// pass it parks the worker. Parking is the milestone that the worker has made
/// every decision due by the horizon, so the test can then submit input at
/// exactly that policy time.
struct HorizonClock {
    state: Mutex<HorizonClockState>,
    changed: Condvar,
}

struct HorizonClockState {
    now: Instant,
    horizon: Instant,
    worker_parked: bool,
}

impl HorizonClock {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            state: Mutex::new(HorizonClockState {
                now,
                horizon: now,
                worker_parked: false,
            }),
            changed: Condvar::new(),
        }
    }

    /// Lets the worker run through `at`, waits until it parks beyond it, and
    /// leaves policy time at exactly `at`. The worker must have queued pages;
    /// an idle worker waits on its condition variable and never parks.
    fn run_until(&self, at: Instant) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Horizon clock lock was poisoned."))?;
        state.horizon = at;
        state.worker_parked = false;
        self.changed.notify_all();
        let (mut state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(1), |state| !state.worker_parked)
            .map_err(|_| AppError::state("Horizon clock lock was poisoned."))?;
        if timeout.timed_out() && !state.worker_parked {
            return Err(AppError::runtime(
                "The publisher worker did not park at the policy-time horizon.",
            ));
        }
        state.now = state.now.max(at);
        Ok(())
    }

    /// Removes the horizon so the worker can drain without further input.
    fn release(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.horizon = state.now + Duration::from_secs(3_600);
            self.changed.notify_all();
        }
    }
}

impl Clock for HorizonClock {
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
        let deadline = state.now + duration;
        while deadline > state.horizon {
            if !state.worker_parked {
                state.worker_parked = true;
                self.changed.notify_all();
            }
            let Ok(next_state) = self.changed.wait(state) else {
                return;
            };
            state = next_state;
        }
        state.now = state.now.max(deadline);
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

    fn wait_for_texts_then_typing_off(&self, text_count: usize) -> AppResult<Vec<TransportEvent>> {
        wait_for_recorded(
            &self.events,
            &self.changed,
            &format!("{text_count} text attempt(s) followed by typing-off"),
            |events| texts_then_typing_off(events, text_count),
        )
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
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.record(TransportEvent::Text(text.as_str().to_string()))?;
        Ok(ChatboxSendReceipt {
            target: "recording".to_string(),
            byte_count: text.as_str().len(),
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

    fn wait_for_texts_then_typing_off(&self, text_count: usize) -> AppResult<Vec<TransportEvent>> {
        self.recording.wait_for_texts_then_typing_off(text_count)
    }
}

impl ChatboxTransport for BlockFirstTextTransport {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.recording
            .record(TransportEvent::Text(text.as_str().to_string()))?;

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
            byte_count: text.as_str().len(),
        })
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.recording.record(TransportEvent::Typing(is_typing))
    }
}

struct BlockTypingReassertTransport {
    recording: RecordingTransport,
    typing_on_attempts: AtomicUsize,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl BlockTypingReassertTransport {
    fn new(entered: mpsc::Sender<()>, release: mpsc::Receiver<()>) -> Self {
        Self {
            recording: RecordingTransport::new(),
            typing_on_attempts: AtomicUsize::new(0),
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(release),
        }
    }

    fn wait_for_events(&self, count: usize) -> AppResult<Vec<TransportEvent>> {
        self.recording.wait_for_events(count)
    }
}

impl ChatboxTransport for BlockTypingReassertTransport {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.recording.send_text(text)
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.recording.record(TransportEvent::Typing(is_typing))?;
        if is_typing && self.typing_on_attempts.fetch_add(1, Ordering::SeqCst) == 1 {
            if let Ok(mut entered) = self.entered.lock()
                && let Some(entered) = entered.take()
            {
                let _ = entered.send(());
            }
            self.release
                .lock()
                .map_err(|_| AppError::state("Blocking transport lock was poisoned."))?
                .recv()
                .map_err(|_| AppError::runtime("Blocking typing reassertion was not released."))?;
        }

        Ok(())
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
    fn new(clock: Arc<dyn Clock>, failed_text_attempts: impl IntoIterator<Item = usize>) -> Self {
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

    fn wait_for_texts_then_typing_off(
        &self,
        text_count: usize,
    ) -> AppResult<Vec<TimedTransportEvent>> {
        wait_for_recorded(
            &self.events,
            &self.changed,
            &format!("{text_count} timed text attempt(s) followed by typing-off"),
            |events| {
                let events = events
                    .iter()
                    .map(|event| event.event.clone())
                    .collect::<Vec<_>>();
                texts_then_typing_off(&events, text_count)
            },
        )
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
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        let attempt = self.next_text_attempt.fetch_add(1, Ordering::SeqCst);
        self.record(TransportEvent::Text(text.as_str().to_string()))?;
        if self.failed_text_attempts.contains(&attempt) {
            return Err(AppError::osc_send(
                "scripted",
                format!("Scripted failure for text attempt {attempt}."),
            ));
        }

        Ok(ChatboxSendReceipt {
            target: "scripted".to_string(),
            byte_count: text.as_str().len(),
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

struct RecordedDiagnostics {
    diagnostics: Mutex<Vec<CompletedPublisherDiagnostic>>,
    changed: Condvar,
}

impl RecordedDiagnostics {
    fn new() -> Self {
        Self {
            diagnostics: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        }
    }

    fn record(&self, diagnostic: CompletedPublisherDiagnostic) {
        if let Ok(mut diagnostics) = self.diagnostics.lock() {
            diagnostics.push(diagnostic);
            self.changed.notify_all();
        }
    }

    fn contains(
        &self,
        predicate: impl Fn(&CompletedPublisherDiagnostic) -> bool,
    ) -> AppResult<bool> {
        self.diagnostics
            .lock()
            .map(|diagnostics| diagnostics.iter().any(predicate))
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))
    }

    fn wait_for(
        &self,
        expectation: &str,
        predicate: impl Fn(&CompletedPublisherDiagnostic) -> bool,
    ) -> AppResult<()> {
        let diagnostics = self
            .diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        let contains_match =
            |diagnostics: &[CompletedPublisherDiagnostic]| diagnostics.iter().any(&predicate);
        let (diagnostics, timeout) = self
            .changed
            .wait_timeout_while(diagnostics, Duration::from_secs(1), |diagnostics| {
                !contains_match(diagnostics)
            })
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;

        if timeout.timed_out() && !contains_match(&diagnostics) {
            return Err(AppError::runtime(format!(
                "Expected {expectation} within one second; observed {} publisher diagnostic(s).",
                diagnostics.len()
            )));
        }

        Ok(())
    }
}

fn recording_reporter() -> (CompletedPublisherReporter, Arc<RecordedDiagnostics>) {
    let diagnostics = Arc::new(RecordedDiagnostics::new());
    let recorded_diagnostics = Arc::clone(&diagnostics);
    let reporter: CompletedPublisherReporter = Arc::new(move |diagnostic| {
        recorded_diagnostics.record(diagnostic);
    });

    (reporter, diagnostics)
}

fn submit_handled(publisher: &CompletedChatboxPublisher, event: SourceUnitEvent) -> AppResult<()> {
    assert_eq!(
        publisher.try_handle_input(event)?,
        PublicationObservationOutcome::Handled
    );
    Ok(())
}

fn prepared_strings(text: &str) -> AppResult<Vec<String>> {
    prepare_completed_pages(text)
        .map(|pages| {
            pages
                .into_iter()
                .map(|page| page.as_str().to_string())
                .collect()
        })
        .map_err(|error| AppError::runtime(describe_layout_error(error)))
}

/// Advances the controlled clock under the publisher state lock, in the
/// worker's lock order (publisher state, then clock). The worker evaluates and
/// enters its timed wait under that lock, so it either observes the new time or
/// is already waiting when the notification arrives; the wakeup cannot be lost.
fn advance_publisher_clock(
    clock: &ControlledClock,
    publisher: &CompletedChatboxPublisher,
    duration: Duration,
) {
    let _state = publisher
        .shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clock.advance(duration);
    publisher.shared.wake.notify_all();
}

fn wait_for_next_typing_reassert(
    clock: &ControlledClock,
    publisher: &CompletedChatboxPublisher,
) -> AppResult<Instant> {
    let state = publisher
        .shared
        .state
        .lock()
        .map_err(|_| AppError::state("Publisher state lock was poisoned."))?;
    let (state, _) = publisher
        .shared
        .wake
        .wait_timeout_while(state, Duration::from_secs(1), |state| {
            state
                .next_typing_reassert_at
                .is_none_or(|deadline| deadline <= clock.now())
        })
        .map_err(|_| AppError::state("Publisher state lock was poisoned."))?;
    let deadline = state.next_typing_reassert_at.ok_or_else(|| {
        AppError::runtime("Publisher did not schedule the next typing reassertion.")
    })?;
    Ok(deadline)
}

fn advance_to_next_typing_reassert(
    clock: &ControlledClock,
    publisher: &CompletedChatboxPublisher,
) -> AppResult<()> {
    let deadline = wait_for_next_typing_reassert(clock, publisher)?;
    advance_publisher_clock(
        clock,
        publisher,
        deadline.saturating_duration_since(clock.now()),
    );
    Ok(())
}

#[test]
fn sends_every_exact_page_in_order() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(AdvancingClock::new());
    let pacer = ChatboxTextPacer::with_clock(clock);
    let committer = open_committer();
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let recorded_diagnostics = Arc::clone(&diagnostics);
    let reporter: CompletedPublisherReporter = Arc::new(move |diagnostic| {
        if let Ok(mut diagnostics) = recorded_diagnostics.lock() {
            diagnostics.push(diagnostic);
        }
    });
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        committer,
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;
    let text = "中".repeat(136);
    let expected_pages = prepared_strings(&text)?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "unit-a".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "unit-a".to_string(),
            revision: 1,
            text,
        },
    )?;

    // Typing reassertions may interleave with the first page's reading dwell,
    // so the milestone is the typing-off that follows every page.
    let events = transport.wait_for_texts_then_typing_off(expected_pages.len())?;
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
        ChatboxTextPacer::with_clock(clock),
        open_committer(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "unit-a".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "unit-a".to_string(),
            revision: 1,
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
            SourceUnitEvent::Opened {
                unit_id: "unit-b".to_string(),
            },
        )?;
        submit_handled(
            &submitted_publisher,
            SourceUnitEvent::Completed {
                unit_id: "unit-b".to_string(),
                revision: 1,
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
fn overload_drops_only_the_oldest_whole_unit_waiting_for_its_first_send_attempt() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let pacer = ChatboxTextPacer::with_clock(clock.clone());
    pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
        .attempt(|| Ok(()))?;
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            // Room for one 136-ideograph unit (an 8-s capped page plus a
            // 1-s page) and one short 1-s unit, but not a second long unit.
            max_resident_reading_time: Duration::from_secs(10),
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    for (unit_id, text) in [
        ("unit-a", "中".repeat(136)),
        ("unit-b", "B".to_string()),
        ("unit-c", "中".repeat(136)),
    ] {
        submit_handled(
            &publisher,
            SourceUnitEvent::Opened {
                unit_id: unit_id.to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            SourceUnitEvent::Completed {
                unit_id: unit_id.to_string(),
                revision: 1,
                text,
            },
        )?;
    }

    clock.release_automatic();
    let events = transport.wait_for_texts_then_typing_off(3)?;
    let sent_pages = events
        .iter()
        .filter_map(|event| match event {
            TransportEvent::Text(text) => Some(text.clone()),
            TransportEvent::Typing(_) => None,
        })
        .collect::<Vec<_>>();
    let mut expected_pages = vec!["B".to_string()];
    expected_pages.extend(prepared_strings(&"中".repeat(136))?);
    assert_eq!(sent_pages, expected_pages);
    assert_eq!(events.first(), Some(&TransportEvent::Typing(true)));
    assert_eq!(events.last(), Some(&TransportEvent::Typing(false)));

    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitDroppedOverload {
            unit_id,
            page_count: 2,
        } if unit_id == "unit-a"
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;

    Ok(())
}

#[test]
fn failed_page_consumes_pacing_starts_no_dwell_and_aborts_the_rest_of_its_unit() -> AppResult<()> {
    let clock = Arc::new(ControlledClock::new());
    let clock_for_transport: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(clock_for_transport, [2]));
    let pacer = ChatboxTextPacer::with_clock(clock.clone());
    pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
        .attempt(|| Ok(()))?;
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;
    let first_text = "中".repeat(271);
    let first_pages = prepared_strings(&first_text)?;
    assert_eq!(first_pages.len(), 3);

    for (unit_id, text) in [("unit-a", first_text), ("unit-b", "B".to_string())] {
        submit_handled(
            &publisher,
            SourceUnitEvent::Opened {
                unit_id: unit_id.to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            SourceUnitEvent::Completed {
                unit_id: unit_id.to_string(),
                revision: 1,
                text,
            },
        )?;
    }
    clock.release_automatic();

    let events = transport.wait_for_texts_then_typing_off(3)?;
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
    // The accepted full first page holds the Chatbox for the capped dwell.
    assert_eq!(
        text_attempts[1]
            .at
            .saturating_duration_since(text_attempts[0].at),
        PROVISIONAL_MAX_PAGE_DWELL
    );
    // The failed attempt displayed nothing new, so only its consumed pacing
    // opportunity separates it from the next unit.
    assert_eq!(
        text_attempts[2]
            .at
            .saturating_duration_since(text_attempts[1].at),
        Duration::from_secs(1)
    );

    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitSendFailed {
            unit_id,
            page_index: 2,
            page_count: 3,
            pages_sent: 1,
            ..
        } if unit_id == "unit-a"
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;

    Ok(())
}

#[test]
fn send_started_unit_is_protected_and_new_unit_is_rejected_without_eviction() -> AppResult<()> {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let transport = Arc::new(BlockFirstTextTransport::new(
        entered_sender,
        release_receiver,
    ));
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            // Room for the in-flight unit's 8-s and 1-s pages plus one short
            // unit; another 9-s unit cannot fit beside the protected pages.
            max_resident_reading_time: Duration::from_secs(10),
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;
    let first_text = "中".repeat(136);
    let first_pages = prepared_strings(&first_text)?;
    assert_eq!(first_pages.len(), 2);

    for (unit_id, text) in [("unit-a", first_text), ("unit-b", "B".to_string())] {
        submit_handled(
            &publisher,
            SourceUnitEvent::Opened {
                unit_id: unit_id.to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            SourceUnitEvent::Completed {
                unit_id: unit_id.to_string(),
                revision: 1,
                text,
            },
        )?;
    }
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("The first unit did not begin its send attempt."))?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "unit-c".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "unit-c".to_string(),
            revision: 1,
            text: "中".repeat(136),
        },
    )?;
    release_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the first in-flight send attempt."))?;

    let events = transport.wait_for_texts_then_typing_off(3)?;
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

    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitRejectedOverload {
            unit_id,
            page_count: 2,
        } if unit_id == "unit-c"
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn unit_longer_than_the_reading_budget_is_rejected_whole_without_changing_the_queue()
-> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let pacer = ChatboxTextPacer::with_clock(clock.clone());
    pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
        .attempt(|| Ok(()))?;
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            // 271 ideographs need 8 s + 8 s + 1 s: longer than the budget
            // even with nothing else queued.
            max_resident_reading_time: Duration::from_secs(10),
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    for (unit_id, text) in [("kept", "A".to_string()), ("oversized", "中".repeat(271))] {
        submit_handled(
            &publisher,
            SourceUnitEvent::Opened {
                unit_id: unit_id.to_string(),
            },
        )?;
        submit_handled(
            &publisher,
            SourceUnitEvent::Completed {
                unit_id: unit_id.to_string(),
                revision: 1,
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitRejectedOverload {
            unit_id,
            page_count: 3,
        } if unit_id == "oversized"
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn stale_unit_waiting_for_its_first_send_attempt_expires_whole() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let pacer = ChatboxTextPacer::with_clock(clock.clone());
    pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
        .attempt(|| Ok(()))?;
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "expired".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "expired".to_string(),
            revision: 1,
            text: "中".repeat(136),
        },
    )?;
    transport.wait_for_events(1)?;
    clock.wait_for_sleep_calls(1)?;
    clock.advance(Duration::from_secs(30));
    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "fresh".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "fresh".to_string(),
            revision: 1,
            text: "fresh".to_string(),
        },
    )?;
    clock.release_automatic();

    assert_eq!(
        transport.wait_for_events(4)?,
        vec![
            TransportEvent::Typing(true),
            // The fake clock advances past the four-second refresh while
            // the fresh unit keeps overall activity continuously active.
            TransportEvent::Typing(true),
            TransportEvent::Text("fresh".to_string()),
            TransportEvent::Typing(false),
        ]
    );
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitExpired {
            unit_id,
            page_count: 2,
        } if unit_id == "expired"
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn overlapping_activity_keeps_typing_on_until_the_last_unit_resolves() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        open_committer(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "unit-a".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "unit-b".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Aborted {
            unit_id: "unit-a".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "unit-b".to_string(),
            revision: 1,
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
fn active_typing_is_reasserted_on_the_best_effort_interval() -> AppResult<()> {
    let clock = Arc::new(ControlledClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock.clone()),
        open_committer(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "long-speech".to_string(),
        },
    )?;

    transport.wait_for_events(1)?;
    for expected_count in 2..=4 {
        advance_to_next_typing_reassert(clock.as_ref(), &publisher)?;
        transport.wait_for_events(expected_count)?;
    }
    let events = transport.wait_for_events(4)?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Aborted {
            unit_id: "long-speech".to_string(),
        },
    )?;
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.clone())
            .collect::<Vec<_>>(),
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
        ]
    );
    assert!(
        events.windows(2).all(|events| {
            events[1].at.duration_since(events[0].at) == TYPING_REASSERT_INTERVAL
        })
    );
    assert_eq!(
        transport
            .wait_for_events(5)?
            .into_iter()
            .map(|event| event.event)
            .collect::<Vec<_>>(),
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(false),
        ]
    );

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn failed_typing_reassertion_waits_before_trying_again() -> AppResult<()> {
    let clock = Arc::new(ControlledClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::with_failures(transport_clock, [], [2]));
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock.clone()),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "typing-refresh-failure".to_string(),
        },
    )?;

    transport.wait_for_events(1)?;
    advance_to_next_typing_reassert(clock.as_ref(), &publisher)?;
    transport.wait_for_events(2)?;
    advance_to_next_typing_reassert(clock.as_ref(), &publisher)?;
    let events = transport.wait_for_events(3)?;
    assert!(
        events.windows(2).all(|events| {
            events[1].at.duration_since(events[0].at) == TYPING_REASSERT_INTERVAL
        })
    );
    assert!(
        events
            .iter()
            .all(|event| event.event == TransportEvent::Typing(true))
    );
    submit_handled(
        &publisher,
        SourceUnitEvent::Aborted {
            unit_id: "typing-refresh-failure".to_string(),
        },
    )?;
    assert_eq!(
        transport
            .wait_for_events(4)?
            .into_iter()
            .map(|event| event.event)
            .collect::<Vec<_>>(),
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(false),
        ]
    );
    diagnostics.wait_for("a failed typing-on diagnostic", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::TypingFailed {
                is_typing: true,
                ..
            }
        )
    })?;
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn stop_cancels_a_pending_typing_reassertion() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let fence = GenerationFence::new();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock.clone()),
        fence.committer(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "stopped-before-refresh".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    wait_for_next_typing_reassert(clock.as_ref(), &publisher)?;
    advance_publisher_clock(clock.as_ref(), &publisher, Duration::from_secs(3));

    close_at_fence(&fence, &publisher)?;
    publisher.join()?;
    close_at_fence(&fence, &publisher)?;
    publisher.join()?;

    assert_eq!(
        transport.events()?,
        vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );
    Ok(())
}

#[test]
fn stop_waits_for_a_linearized_typing_reassertion_then_cleans_up() -> AppResult<()> {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let transport = Arc::new(BlockTypingReassertTransport::new(
        entered_sender,
        release_receiver,
    ));
    let clock = Arc::new(ControlledClock::new());
    let fence = GenerationFence::new();
    let committer = fence.committer();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock.clone()),
        committer.clone(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "typing-stop-race".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    advance_to_next_typing_reassert(clock.as_ref(), &publisher)?;
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Typing reassertion did not reach transport."))?;

    let stop_fence = fence.clone();
    let stop_publisher = publisher.clone();
    let (stop_finished_sender, stop_finished_receiver) = mpsc::channel();
    let stop = thread::spawn(move || {
        let result = close_at_fence(&stop_fence, &stop_publisher);
        let _ = stop_finished_sender.send(());
        result
    });

    wait_for_commits_closed(&committer)?;
    assert!(matches!(
        stop_finished_receiver.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    release_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the typing reassertion."))?;
    stop_finished_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Stop did not finish after the typing attempt."))?;
    stop.join()
        .map_err(|_| AppError::runtime("Stop test thread panicked."))??;
    publisher.join()?;

    assert_eq!(
        transport.wait_for_events(3)?,
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(false),
        ]
    );
    Ok(())
}

#[test]
fn typing_is_reasserted_through_page_dwell_without_shifting_text_attempts() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    // Six full 135-ideograph pages, each dwelling for the cap, then one more.
    let text = "中".repeat(811);
    let page_count = prepare_completed_pages(&text)
        .map_err(|error| AppError::runtime(describe_layout_error(error)))?
        .len();
    assert_eq!(page_count, 7);
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock),
        open_committer(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "paced-around-typing".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "paced-around-typing".to_string(),
            revision: 1,
            text,
        },
    )?;

    let events = transport.wait_for_texts_then_typing_off(page_count)?;
    let text_attempts = events
        .iter()
        .filter_map(|event| match event.event {
            TransportEvent::Text(_) => Some(event.at),
            TransportEvent::Typing(_) => None,
        })
        .collect::<Vec<_>>();
    let typing_on = events
        .iter()
        .filter(|event| event.event == TransportEvent::Typing(true))
        .map(|event| event.at)
        .collect::<Vec<_>>();
    assert_eq!(text_attempts.len(), page_count);
    // Reassertions during each hold neither consume nor shift a text attempt.
    assert!(
        text_attempts
            .windows(2)
            .all(|attempts| attempts[1].duration_since(attempts[0]) == PROVISIONAL_MAX_PAGE_DWELL)
    );
    // Typing stays on while pages remain queued and is reasserted on its own
    // interval throughout every dwell, then turns off after the final page.
    assert_eq!(typing_on.first(), text_attempts.first());
    assert!(
        typing_on
            .windows(2)
            .all(|typing| typing[1].duration_since(typing[0]) == TYPING_REASSERT_INTERVAL)
    );
    assert_eq!(typing_on.last(), text_attempts.last());
    assert_eq!(
        events.last().map(|event| event.at),
        text_attempts.last().copied()
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
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "layout-failure".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    let oversized_grapheme = format!("a{}", "\u{301}".repeat(144));
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "layout-failure".to_string(),
            revision: 1,
            text: oversized_grapheme,
        },
    )?;

    assert_eq!(
        transport.wait_for_events(2)?,
        vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );
    diagnostics.wait_for(
        "a layout-failure diagnostic for layout-failure",
        |diagnostic| {
            matches!(
                diagnostic,
                CompletedPublisherDiagnostic::LayoutFailed { unit_id, .. }
                    if unit_id == "layout-failure"
            )
        },
    )?;

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
        ChatboxTextPacer::with_clock(clock),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "typing-failure".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "typing-failure".to_string(),
            revision: 1,
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::TypingFailed {
            is_typing: true,
            ..
        }
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn stop_interrupts_a_pacing_wait_discards_late_submissions_and_cleans_typing_once() -> AppResult<()>
{
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let pacer = ChatboxTextPacer::with_clock(clock.clone());
    pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
        .attempt(|| Ok(()))?;
    let fence = GenerationFence::new();
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        fence.committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "stopped".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "stopped".to_string(),
            revision: 1,
            text: "must not send".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    clock.wait_for_sleep_calls(1)?;

    close_at_fence(&fence, &publisher)?;
    assert_eq!(
        publisher.try_handle_input(SourceUnitEvent::Completed {
            unit_id: "late".to_string(),
            revision: 1,
            text: "late".to_string(),
        })?,
        PublicationObservationOutcome::Closed
    );
    clock.release_automatic();
    publisher.join()?;
    close_at_fence(&fence, &publisher)?;
    publisher.join()?;

    assert_eq!(
        transport.events()?,
        vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );
    assert_eq!(clock.total_sleep()?, Duration::from_millis(100));
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::PagesDiscardedOnClose {
            reason: PublisherCloseReason::Stop,
            unit_count: 1,
            page_count: 1,
            send_started_unit_count: 0,
            translation_wait_unit_count: 0,
        }
    ))?);

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
    let fence = GenerationFence::new();
    let committer = fence.committer();
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        committer.clone(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;
    let pages = prepared_strings(&"中".repeat(136))?;

    submit_handled(
        &publisher,
        SourceUnitEvent::Opened {
            unit_id: "in-flight".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        SourceUnitEvent::Completed {
            unit_id: "in-flight".to_string(),
            revision: 1,
            text: "中".repeat(136),
        },
    )?;
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("OSC attempt did not reach transport."))?;

    let stop_fence = fence.clone();
    let stop_publisher = publisher.clone();
    let (stop_finished_sender, stop_finished_receiver) = mpsc::channel();
    let stop = thread::spawn(move || -> AppResult<()> {
        let result = close_at_fence(&stop_fence, &stop_publisher);
        let _ = stop_finished_sender.send(());
        result
    });

    wait_for_commits_closed(&committer)?;
    assert!(matches!(
        stop_finished_receiver.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        publisher.try_handle_input(SourceUnitEvent::Completed {
            unit_id: "late".to_string(),
            revision: 1,
            text: "late".to_string(),
        })?,
        PublicationObservationOutcome::Closed
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::PagesDiscardedOnClose {
            reason: PublisherCloseReason::Stop,
            page_count: 1,
            send_started_unit_count: 1,
            ..
        }
    ))?);

    Ok(())
}

#[test]
fn concurrent_close_and_join_perform_one_cleanup() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        open_committer(),
        ContentSelection::SourceOnly,
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
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
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            max_wait_before_first_send_attempt: Duration::from_secs(30),
            max_wait_for_translation: Duration::from_secs(20),
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::WorkerFailed { .. }
    ))?);

    Ok(())
}

// ---------------------------------------------------------------------------
// Reading dwell: an accepted page holds the Chatbox before the next page.
// ---------------------------------------------------------------------------

fn source_unit(text: &str) -> (String, String) {
    (text.to_string(), String::new())
}

fn timed_text_attempts(events: &[TimedTransportEvent]) -> Vec<(String, Instant)> {
    events
        .iter()
        .filter_map(|event| match &event.event {
            TransportEvent::Text(text) => Some((text.clone(), event.at)),
            TransportEvent::Typing(_) => None,
        })
        .collect()
}

fn gaps_between(attempts: &[(String, Instant)]) -> Vec<Duration> {
    attempts
        .windows(2)
        .map(|pair| pair[1].1.saturating_duration_since(pair[0].1))
        .collect()
}

/// Publishes each `(source, translation)` unit under `content` and returns the
/// timed text attempts after every expected page and the final typing-off.
/// Policy time advances only while the worker sleeps, so each gap between
/// attempts is exactly the hold the worker chose, whatever the real thread
/// schedule; Source-only units ignore their Translation.
fn publish_and_time_units(
    content: ContentSelection,
    units: &[(String, String)],
    expected_text_count: usize,
) -> AppResult<Vec<(String, Instant)>> {
    let clock = Arc::new(AdvancingClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock),
        open_committer(),
        content,
        Arc::new(|_| {}),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    for (index, (source, translation)) in units.iter().enumerate() {
        let unit_id = format!("unit-{index}");
        complete_source(&publisher, &unit_id, 1, source)?;
        if content != ContentSelection::SourceOnly {
            complete_translation(&publisher, &unit_id, 1, translation)?;
        }
    }
    let events = transport.wait_for_texts_then_typing_off(expected_text_count)?;

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(timed_text_attempts(&events))
}

#[test]
fn short_page_dwells_only_for_the_one_second_pacing_floor() -> AppResult<()> {
    // Two Latin letters read in well under a second, so the next unit replaces
    // the page as soon as the shared pacer allows, exactly as before dwell.
    let attempts = publish_and_time_units(
        ContentSelection::SourceOnly,
        &[source_unit("ok"), source_unit("next")],
        2,
    )?;

    assert_eq!(gaps_between(&attempts), vec![Duration::from_secs(1)]);
    Ok(())
}

#[test]
fn page_dwell_is_proportional_to_its_reading_length() -> AppResult<()> {
    // Each ideograph reads as one em at 120 ms, so 25 ideographs hold the
    // Chatbox for 3 s and twice as many for 6 s.
    let attempts = publish_and_time_units(
        ContentSelection::SourceOnly,
        &[
            source_unit(&"中".repeat(25)),
            source_unit(&"中".repeat(50)),
            source_unit("next"),
        ],
        3,
    )?;

    assert_eq!(
        gaps_between(&attempts),
        vec![Duration::from_secs(3), Duration::from_secs(6)]
    );
    Ok(())
}

#[test]
fn full_page_dwell_stops_at_the_cap() -> AppResult<()> {
    // A full 135-ideograph page would need 16.2 s at the reading rate.
    let full_page = "中".repeat(135);
    assert_eq!(prepared_strings(&full_page)?.len(), 1);

    let attempts = publish_and_time_units(
        ContentSelection::SourceOnly,
        &[source_unit(&full_page), source_unit("next")],
        2,
    )?;

    assert_eq!(gaps_between(&attempts), vec![PROVISIONAL_MAX_PAGE_DWELL]);
    Ok(())
}

#[test]
fn final_page_of_a_unit_dwells_before_the_next_unit_replaces_it() -> AppResult<()> {
    let long_unit = "中".repeat(160);
    let pages = prepared_strings(&long_unit)?;
    assert_eq!(
        pages
            .iter()
            .map(|page| page.chars().count())
            .collect::<Vec<_>>(),
        vec![135, 25]
    );

    let attempts = publish_and_time_units(
        ContentSelection::SourceOnly,
        &[source_unit(&long_unit), source_unit("next")],
        3,
    )?;

    assert_eq!(
        attempts
            .iter()
            .map(|(text, _)| text.clone())
            .collect::<Vec<_>>(),
        vec![pages[0].clone(), pages[1].clone(), "next".to_string()]
    );
    // The full first page dwells for the cap; the 25-ideograph final page
    // still holds the Chatbox for its own 3 s before the next unit.
    assert_eq!(
        gaps_between(&attempts),
        vec![PROVISIONAL_MAX_PAGE_DWELL, Duration::from_secs(3)]
    );
    Ok(())
}

#[test]
fn bilingual_page_dwells_for_the_full_content_of_both_lanes() -> AppResult<()> {
    let source = "中".repeat(10);
    let translation = "文".repeat(15);

    let attempts = publish_and_time_units(
        ContentSelection::Bilingual,
        &[
            (source.clone(), translation.clone()),
            ("next".to_string(), "下".to_string()),
        ],
        2,
    )?;

    assert_eq!(
        attempts.first().map(|(text, _)| text.clone()),
        Some(format!("{source}\n{translation}"))
    );
    // The shared page carries 25 ideographs and holds for 3 s; either lane
    // alone would have held it for only 1.2 s or 1.8 s.
    assert_eq!(gaps_between(&attempts), vec![Duration::from_secs(3)]);
    Ok(())
}

#[test]
fn translation_only_page_dwells_for_the_published_translation() -> AppResult<()> {
    let translation = "文".repeat(25);

    // The held 50-ideograph Source would read for 6 s, but only its
    // 25-ideograph Translation is published.
    let attempts = publish_and_time_units(
        ContentSelection::TranslationOnly,
        &[
            ("中".repeat(50), translation.clone()),
            ("next".to_string(), "下".to_string()),
        ],
        2,
    )?;

    assert_eq!(
        attempts
            .iter()
            .map(|(text, _)| text.clone())
            .collect::<Vec<_>>(),
        vec![translation, "下".to_string()]
    );
    assert_eq!(gaps_between(&attempts), vec![Duration::from_secs(3)]);
    Ok(())
}

#[test]
fn stop_during_a_page_dwell_sends_nothing_further_and_discards_queued_pages() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let fence = GenerationFence::new();
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock.clone()),
        fence.committer(),
        ContentSelection::SourceOnly,
        reporter,
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;
    let long_unit = "中".repeat(160);
    let pages = prepared_strings(&long_unit)?;

    complete_source(&publisher, "dwelling", 1, &long_unit)?;
    complete_source(&publisher, "queued", 1, "next")?;
    // The fresh pacer admits the first page without sleeping, so the first
    // clock sleep is the worker holding the next page for the 8-s dwell.
    assert_eq!(
        transport.wait_for_events(2)?,
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Text(pages[0].clone()),
        ]
    );
    clock.wait_for_sleep_calls(1)?;

    close_at_fence(&fence, &publisher)?;
    clock.release_automatic();
    publisher.join()?;

    assert_eq!(
        transport.events()?,
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Text(pages[0].clone()),
            TransportEvent::Typing(false),
        ]
    );
    // Stop ended the hold at its first poll instead of sleeping through it.
    assert_eq!(clock.total_sleep()?, Duration::from_millis(100));
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::PagesDiscardedOnClose {
            reason: PublisherCloseReason::Stop,
            unit_count: 2,
            page_count: 2,
            send_started_unit_count: 1,
            translation_wait_unit_count: 0,
        }
    ))?);
    Ok(())
}

#[test]
fn unit_expiring_during_a_page_dwell_is_dropped_whole_at_its_deadline() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    let (reporter, diagnostics) = recording_reporter();
    let first_send_budget = Duration::from_secs(5);
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        PublisherLimits {
            max_resident_reading_time: PROVISIONAL_MAX_RESIDENT_READING_TIME,
            // Shorter than one capped dwell, so the waiting unit expires while
            // the displayed page still holds the Chatbox.
            max_wait_before_first_send_attempt: first_send_budget,
            max_wait_for_translation: Duration::from_secs(20),
        },
    )?;
    let displayed = "中".repeat(135);

    // Both units open first, so typing stays on until the second resolves.
    for unit_id in ["displayed", "stale"] {
        submit_handled(
            &publisher,
            SourceUnitEvent::Opened {
                unit_id: unit_id.to_string(),
            },
        )?;
    }
    for (unit_id, text) in [
        ("displayed", displayed.clone()),
        ("stale", "stale".to_string()),
    ] {
        submit_handled(
            &publisher,
            SourceUnitEvent::Completed {
                unit_id: unit_id.to_string(),
                revision: 1,
                text,
            },
        )?;
    }

    let events = transport.wait_for_texts_then_typing_off(1)?;
    let attempts = timed_text_attempts(&events);
    let [(sent_text, sent_at)] = attempts.as_slice() else {
        return Err(AppError::runtime("Expected exactly one text attempt."));
    };
    assert_eq!(sent_text, &displayed);
    // The hold wakes for the waiting unit's first-send budget: the unit is
    // dropped whole and typing released then, not when the 8-s dwell ends.
    assert_eq!(
        events.last().map(|event| event.at),
        Some(*sent_at + first_send_budget)
    );
    diagnostics.wait_for("the stale unit to expire whole", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::UnitExpired {
                unit_id,
                page_count: 1,
            } if unit_id == "stale"
        )
    })?;

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn reading_budget_drops_the_oldest_waiting_unit_although_few_pages_are_resident() -> AppResult<()> {
    let transport = Arc::new(RecordingTransport::new());
    let clock = Arc::new(ControlledClock::new());
    let pacer = ChatboxTextPacer::with_clock(clock.clone());
    pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::runtime("Initial pacing reservation was cancelled."))?
        .attempt(|| Ok(()))?;
    let (reporter, diagnostics) = recording_reporter();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
        content_limits(PROVISIONAL_MAX_PAGE_DWELL * 2),
    )?;
    // Only three pages are resident, but each dwells for the cap: 24 s of
    // reading cannot fit a 16-s budget, so the oldest waiting unit goes whole.
    let [oldest, middle, newest] = ["甲", "乙", "丙"].map(|glyph| glyph.repeat(70));

    complete_source(&publisher, "oldest", 1, &oldest)?;
    complete_source(&publisher, "middle", 1, &middle)?;
    complete_source(&publisher, "newest", 1, &newest)?;
    clock.release_automatic();

    let events = transport.wait_for_texts_then_typing_off(2)?;
    assert_eq!(sent_texts(&events), vec![middle, newest]);
    diagnostics.wait_for("the oldest unit to be dropped whole", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::UnitDroppedOverload {
                unit_id,
                page_count: 1,
            } if unit_id == "oldest"
        )
    })?;

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn sustained_speech_faster_than_reading_drops_whole_units_and_bounds_staleness() -> AppResult<()> {
    const ARRIVAL_INTERVAL: Duration = Duration::from_secs(4);
    let clock = Arc::new(HorizonClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    let (reporter, diagnostics) = recording_reporter();
    // The production provisional limits: a 64-s reading budget and a 30-s
    // first-send budget.
    let publisher = CompletedChatboxPublisher::start(
        transport.clone(),
        ChatboxTextPacer::with_clock(clock.clone()),
        open_committer(),
        ContentSelection::SourceOnly,
        reporter,
    )?;
    let started_at = clock.now();
    // A two-page opening utterance, then one utterance every 4 s. Every page
    // dwells for the 8-s cap, so speech arrives twice as fast as it is read.
    let texts = [
        "甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸", "子", "丑",
    ]
    .iter()
    .enumerate()
    .map(|(index, glyph)| glyph.repeat(if index == 0 { 205 } else { 70 }))
    .collect::<Vec<_>>();
    let mut arrivals = Vec::new();

    for (index, text) in texts.iter().enumerate() {
        let offset = u32::try_from(index)
            .map_err(|_| AppError::state("Scripted arrival index overflowed."))?;
        let arrival = started_at + ARRIVAL_INTERVAL * offset;
        if index > 0 {
            // Each arrival lands while earlier pages are still dwelling.
            clock.run_until(arrival)?;
        }
        complete_source(&publisher, &format!("unit-{index}"), 1, text)?;
        arrivals.push(arrival);
    }
    clock.release();

    let events = transport.wait_for_texts_then_typing_off(10)?;
    let attempts = timed_text_attempts(&events);
    let opening_pages = prepared_strings(&texts[0])?;
    let published_units = [1, 2, 3, 4, 5, 7, 9, 11];
    let mut expected_texts = opening_pages;
    expected_texts.extend(published_units.iter().map(|&index| texts[index].clone()));
    assert_eq!(
        attempts
            .iter()
            .map(|(text, _)| text.clone())
            .collect::<Vec<_>>(),
        expected_texts
    );
    // The queue drains at one capped page every 8 s, in arrival order.
    assert!(
        gaps_between(&attempts)
            .iter()
            .all(|gap| *gap == PROVISIONAL_MAX_PAGE_DWELL)
    );
    // Every published utterance starts within the first-send budget; the
    // oldest waiting utterances that could not are dropped whole instead of
    // making every later one staler.
    for (&index, (_, sent_at)) in published_units.iter().zip(attempts.iter().skip(2)) {
        assert!(
            sent_at.saturating_duration_since(arrivals[index])
                < PROVISIONAL_MAX_WAIT_BEFORE_FIRST_SEND_ATTEMPT
        );
    }
    for expired in ["unit-6", "unit-8", "unit-10"] {
        diagnostics.wait_for("an expired utterance dropped whole", |diagnostic| {
            matches!(
                diagnostic,
                CompletedPublisherDiagnostic::UnitExpired {
                    unit_id,
                    page_count: 1,
                } if unit_id == expired
            )
        })?;
    }
    diagnostics.wait_for("the final utterance to be sent", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::UnitSendSucceeded { unit_id, .. } if unit_id == "unit-11"
        )
    })?;
    // Expiry, not the reading budget, trimmed this backlog.
    assert!(!diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitDroppedOverload { .. }
            | CompletedPublisherDiagnostic::UnitRejectedOverload { .. }
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Selected-content publication: held Source units, exact pairing, wait budget.
// ---------------------------------------------------------------------------

struct ContentPublisher {
    publisher: CompletedChatboxPublisher,
    transport: Arc<RecordingTransport>,
    diagnostics: Arc<RecordedDiagnostics>,
    fence: GenerationFence,
}

fn content_limits(max_resident_reading_time: Duration) -> PublisherLimits {
    PublisherLimits {
        max_resident_reading_time,
        max_wait_before_first_send_attempt: Duration::from_secs(30),
        max_wait_for_translation: Duration::from_secs(20),
    }
}

fn start_content_publisher(
    content: ContentSelection,
    pacer: ChatboxTextPacer,
    limits: PublisherLimits,
) -> AppResult<ContentPublisher> {
    let transport = Arc::new(RecordingTransport::new());
    let (reporter, diagnostics) = recording_reporter();
    let fence = GenerationFence::new();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        pacer,
        fence.committer(),
        content,
        reporter,
        limits,
    )?;
    Ok(ContentPublisher {
        publisher,
        transport,
        diagnostics,
        fence,
    })
}

fn advancing_pacer() -> ChatboxTextPacer {
    ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new()))
}

fn held_source_ref(unit_id: &str, revision: u64) -> SourceSnapshotRef {
    SourceSnapshotRef {
        generation: 1,
        stream_id: "recognition-1-1".to_string(),
        unit_id: unit_id.to_string(),
        revision,
    }
}

fn complete_source(
    publisher: &CompletedChatboxPublisher,
    unit_id: &str,
    revision: u64,
    text: &str,
) -> AppResult<()> {
    submit_handled(
        publisher,
        SourceUnitEvent::Opened {
            unit_id: unit_id.to_string(),
        },
    )?;
    submit_handled(
        publisher,
        SourceUnitEvent::Completed {
            unit_id: unit_id.to_string(),
            revision,
            text: text.to_string(),
        },
    )
}

fn complete_translation(
    publisher: &CompletedChatboxPublisher,
    unit_id: &str,
    revision: u64,
    text: &str,
) -> AppResult<()> {
    submit_handled(
        publisher,
        SourceUnitEvent::TranslationCompleted {
            source_ref: held_source_ref(unit_id, revision),
            text: text.to_string(),
        },
    )
}

fn fail_translation(
    publisher: &CompletedChatboxPublisher,
    unit_id: &str,
    revision: u64,
    reason_code: TranslationFailureReason,
) -> AppResult<()> {
    submit_handled(
        publisher,
        SourceUnitEvent::TranslationFailed {
            source_ref: held_source_ref(unit_id, revision),
            reason_code,
        },
    )
}

fn sent_texts(events: &[TransportEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            TransportEvent::Text(text) => Some(text.clone()),
            TransportEvent::Typing(_) => None,
        })
        .collect()
}

fn bilingual_strings(source: &str, translation: &str) -> AppResult<Vec<String>> {
    prepare_bilingual_completed_pages(source, translation)
        .map(|pages| {
            pages
                .into_iter()
                .map(|page| page.into_prepared_text().as_str().to_string())
                .collect()
        })
        .map_err(|error| AppError::runtime(describe_layout_error(error)))
}

const EVERY_TRANSLATION_FAILURE_REASON: [TranslationFailureReason; 12] = [
    TranslationFailureReason::ProviderAuthenticationFailed,
    TranslationFailureReason::ProviderPermissionDenied,
    TranslationFailureReason::ProviderInvalidRequest,
    TranslationFailureReason::ProviderRateLimited,
    TranslationFailureReason::ProviderUsageLimit,
    TranslationFailureReason::ProviderUnavailable,
    TranslationFailureReason::InvalidOutput,
    TranslationFailureReason::DeadlineExceeded,
    TranslationFailureReason::Backpressure,
    TranslationFailureReason::SourceTooLarge,
    TranslationFailureReason::Stopped,
    TranslationFailureReason::Failed,
];

#[test]
fn translation_only_holds_the_source_and_sends_only_the_exact_translation() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        ..
    } = start_content_publisher(
        ContentSelection::TranslationOnly,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    complete_source(&publisher, "unit-a", 1, "source a")?;
    // Typing stays on while the exact Translation is pending; nothing is sent.
    assert_eq!(
        transport.wait_for_events(1)?,
        vec![TransportEvent::Typing(true)]
    );

    complete_translation(&publisher, "unit-a", 1, "译文 A")?;
    assert_eq!(
        transport.wait_for_events(3)?,
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Text("译文 A".to_string()),
            TransportEvent::Typing(false),
        ]
    );

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn translation_only_preserves_source_admission_order_across_out_of_order_results() -> AppResult<()>
{
    let ContentPublisher {
        publisher,
        transport,
        ..
    } = start_content_publisher(
        ContentSelection::TranslationOnly,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    complete_source(&publisher, "unit-a", 1, "source a")?;
    complete_source(&publisher, "unit-b", 1, "source b")?;
    // The later unit resolves first but must wait behind the held head.
    complete_translation(&publisher, "unit-b", 1, "译文 B")?;
    complete_translation(&publisher, "unit-a", 1, "译文 A")?;

    let events = transport.wait_for_events(4)?;
    assert_eq!(
        sent_texts(&events),
        vec!["译文 A".to_string(), "译文 B".to_string()]
    );
    assert_eq!(events.first(), Some(&TransportEvent::Typing(true)));
    assert_eq!(events.last(), Some(&TransportEvent::Typing(false)));

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn translation_only_omits_every_terminal_failure_and_releases_the_queue_head() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::TranslationOnly,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    for (index, reason) in EVERY_TRANSLATION_FAILURE_REASON.into_iter().enumerate() {
        let unit_id = format!("failed-{index}");
        complete_source(&publisher, &unit_id, 1, &format!("source {index}"))?;
        fail_translation(&publisher, &unit_id, 1, reason)?;
        diagnostics.wait_for(
            "an omitted-unit diagnostic carrying the stable failure reason",
            |diagnostic| {
                matches!(
                    diagnostic,
                    CompletedPublisherDiagnostic::UnitOmittedWithoutTranslation {
                        unit_id: omitted,
                        resolution,
                    } if omitted == &unit_id && *resolution == TranslationResolution::Failed(reason)
                )
            },
        )?;
    }

    // Every failed head released its position: the next exact Translation is
    // the first and only text the transport ever receives.
    complete_source(&publisher, "unit-last", 1, "source last")?;
    complete_translation(&publisher, "unit-last", 1, "最后")?;
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert_eq!(sent_texts(&transport.events()?), vec!["最后".to_string()]);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn bilingual_sends_the_exact_pair_through_every_bilingual_page() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::Bilingual,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;
    let source = "source lane ".repeat(40);
    let translation = "短译文";
    let expected_pages = bilingual_strings(&source, translation)?;
    assert!(expected_pages.len() > 1);

    complete_source(&publisher, "unit-a", 1, &source)?;
    complete_translation(&publisher, "unit-a", 1, translation)?;

    // The queue is the causal text barrier; typing reassertions may interleave
    // with a long unit, so compare the sent pages after quiescence.
    diagnostics.wait_for("the pair to be sent completely", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::UnitSendSucceeded { unit_id, page_count, .. }
                if unit_id == "unit-a" && *page_count == expected_pages.len()
        )
    })?;
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert_eq!(sent_texts(&transport.events()?), expected_pages);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn bilingual_publishes_source_alone_after_failure_and_keeps_pairing_later_units() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::Bilingual,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    complete_source(&publisher, "unit-a", 1, "source a")?;
    complete_source(&publisher, "unit-b", 1, "source b")?;
    fail_translation(
        &publisher,
        "unit-a",
        1,
        TranslationFailureReason::DeadlineExceeded,
    )?;
    complete_translation(&publisher, "unit-b", 1, "译文 B")?;

    let events = transport.wait_for_events(4)?;
    assert_eq!(
        sent_texts(&events),
        vec!["source a".to_string(), "source b\n译文 B".to_string()]
    );
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitQueuedWithoutTranslation {
            unit_id,
            resolution: TranslationResolution::Failed(TranslationFailureReason::DeadlineExceeded),
        } if unit_id == "unit-a"
    ))?);

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn bilingual_layout_failure_falls_back_to_the_exact_source() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::Bilingual,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;
    let oversized_grapheme = format!("a{}", "\u{301}".repeat(144));

    complete_source(&publisher, "unit-a", 1, "readable source")?;
    complete_translation(&publisher, "unit-a", 1, &oversized_grapheme)?;
    let events = transport.wait_for_events(3)?;
    assert_eq!(sent_texts(&events), vec!["readable source".to_string()]);
    diagnostics.wait_for(
        "a Source-only fallback after a layout failure",
        |diagnostic| {
            matches!(
                diagnostic,
                CompletedPublisherDiagnostic::UnitQueuedWithoutTranslation {
                    unit_id,
                    resolution: TranslationResolution::LayoutFailed { .. },
                } if unit_id == "unit-a"
            )
        },
    )?;

    // A Source that cannot be laid out itself is not sent in any form.
    complete_source(&publisher, "unit-b", 1, &oversized_grapheme)?;
    complete_translation(&publisher, "unit-b", 1, "译文 B")?;
    diagnostics.wait_for("a layout-failure diagnostic for unit-b", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::LayoutFailed { unit_id, .. } if unit_id == "unit-b"
        )
    })?;
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert_eq!(
        sent_texts(&transport.events()?),
        vec!["readable source".to_string()]
    );

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn translation_only_wait_budget_omits_the_unit_and_ignores_a_late_result() -> AppResult<()> {
    let clock = Arc::new(ControlledClock::new());
    clock.release_automatic();
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::TranslationOnly,
        ChatboxTextPacer::with_clock(clock.clone()),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    complete_source(&publisher, "unit-a", 1, "source a")?;
    assert_eq!(
        transport.wait_for_events(1)?,
        vec![TransportEvent::Typing(true)]
    );

    advance_publisher_clock(&clock, &publisher, Duration::from_secs(20));
    diagnostics.wait_for("a wait-expired omission for unit-a", |diagnostic| {
        matches!(
            diagnostic,
            CompletedPublisherDiagnostic::UnitOmittedWithoutTranslation {
                unit_id,
                resolution: TranslationResolution::WaitExpired,
            } if unit_id == "unit-a"
        )
    })?;
    assert_eq!(
        transport.wait_for_events(2)?,
        vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );

    // The late result finds no held unit and is a successful no-op.
    complete_translation(&publisher, "unit-a", 1, "迟到的译文")?;
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert!(sent_texts(&transport.events()?).is_empty());

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn bilingual_wait_budget_publishes_the_source_alone() -> AppResult<()> {
    let clock = Arc::new(ControlledClock::new());
    clock.release_automatic();
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::Bilingual,
        ChatboxTextPacer::with_clock(clock.clone()),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    complete_source(&publisher, "unit-a", 1, "source a")?;
    assert_eq!(
        transport.wait_for_events(1)?,
        vec![TransportEvent::Typing(true)]
    );

    advance_publisher_clock(&clock, &publisher, Duration::from_secs(20));
    let events = transport.wait_for_events(3)?;
    assert_eq!(sent_texts(&events), vec!["source a".to_string()]);
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        CompletedPublisherDiagnostic::UnitQueuedWithoutTranslation {
            unit_id,
            resolution: TranslationResolution::WaitExpired,
        } if unit_id == "unit-a"
    ))?);

    complete_translation(&publisher, "unit-a", 1, "迟到的译文")?;
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert_eq!(
        sent_texts(&transport.events()?),
        vec!["source a".to_string()]
    );

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn resolved_translation_that_cannot_fit_is_rejected_whole() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        diagnostics,
        ..
    } = start_content_publisher(
        ContentSelection::TranslationOnly,
        advancing_pacer(),
        // Two capped pages of reading time; the Translation needs three.
        content_limits(PROVISIONAL_MAX_PAGE_DWELL * 2),
    )?;
    let oversized_translation = "中".repeat(400);
    let page_count = prepared_strings(&oversized_translation)?.len();
    assert!(page_count > 2);

    complete_source(&publisher, "unit-a", 1, "source a")?;
    assert_eq!(
        transport.wait_for_events(1)?,
        vec![TransportEvent::Typing(true)]
    );
    complete_translation(&publisher, "unit-a", 1, &oversized_translation)?;
    diagnostics.wait_for(
        "an overload rejection for the resolved unit",
        |diagnostic| {
            matches!(
                diagnostic,
                CompletedPublisherDiagnostic::UnitRejectedOverload {
                    unit_id,
                    page_count: rejected,
                } if unit_id == "unit-a" && *rejected == page_count
            )
        },
    )?;
    assert_eq!(
        transport.wait_for_events(2)?,
        vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn mismatched_translation_does_not_resolve_a_held_unit() -> AppResult<()> {
    let ContentPublisher {
        publisher,
        transport,
        ..
    } = start_content_publisher(
        ContentSelection::TranslationOnly,
        advancing_pacer(),
        content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
    )?;

    complete_source(&publisher, "unit-a", 1, "source a")?;
    // A different Source revision and a different unit must not release the slot.
    complete_translation(&publisher, "unit-a", 2, "错误的修订")?;
    complete_translation(&publisher, "other", 1, "错误的单元")?;
    complete_translation(&publisher, "unit-a", 1, "正确")?;

    assert_eq!(
        transport.wait_for_events(3)?,
        vec![
            TransportEvent::Typing(true),
            TransportEvent::Text("正确".to_string()),
            TransportEvent::Typing(false),
        ]
    );

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn close_discards_held_units_and_rejects_late_results() -> AppResult<()> {
    for reason in [
        PublisherCloseReason::Stop,
        PublisherCloseReason::RuntimeError,
    ] {
        let ContentPublisher {
            publisher,
            transport,
            diagnostics,
            fence,
        } = start_content_publisher(
            ContentSelection::Bilingual,
            advancing_pacer(),
            content_limits(PROVISIONAL_MAX_RESIDENT_READING_TIME),
        )?;

        complete_source(&publisher, "unit-a", 1, "source a")?;
        assert_eq!(
            transport.wait_for_events(1)?,
            vec![TransportEvent::Typing(true)]
        );

        match reason {
            PublisherCloseReason::Stop => close_at_fence(&fence, &publisher)?,
            PublisherCloseReason::RuntimeError => publisher.request_close(reason)?,
        }
        publisher.join()?;

        assert_eq!(
            transport.events()?,
            vec![TransportEvent::Typing(true), TransportEvent::Typing(false)]
        );
        assert!(diagnostics.contains(|diagnostic| matches!(
            diagnostic,
            CompletedPublisherDiagnostic::PagesDiscardedOnClose {
                reason: discarded_reason,
                unit_count: 1,
                page_count: 0,
                send_started_unit_count: 0,
                translation_wait_unit_count: 1,
            } if *discarded_reason == reason
        ))?);
        assert_eq!(
            publisher.try_handle_input(SourceUnitEvent::TranslationCompleted {
                source_ref: held_source_ref("unit-a", 1),
                text: "迟到的译文".to_string(),
            })?,
            PublicationObservationOutcome::Closed
        );
    }
    Ok(())
}
