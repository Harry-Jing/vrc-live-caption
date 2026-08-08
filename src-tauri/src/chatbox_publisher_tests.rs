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
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
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

struct RecordedDiagnostics {
    diagnostics: Mutex<Vec<PublisherDiagnostic>>,
    changed: Condvar,
}

impl RecordedDiagnostics {
    fn new() -> Self {
        Self {
            diagnostics: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        }
    }

    fn record(&self, diagnostic: PublisherDiagnostic) {
        if let Ok(mut diagnostics) = self.diagnostics.lock() {
            diagnostics.push(diagnostic);
            self.changed.notify_all();
        }
    }

    fn contains(&self, predicate: impl Fn(&PublisherDiagnostic) -> bool) -> AppResult<bool> {
        self.diagnostics
            .lock()
            .map(|diagnostics| diagnostics.iter().any(predicate))
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))
    }

    fn wait_for(
        &self,
        expectation: &str,
        predicate: impl Fn(&PublisherDiagnostic) -> bool,
    ) -> AppResult<()> {
        let diagnostics = self
            .diagnostics
            .lock()
            .map_err(|_| AppError::state("Publisher diagnostics lock was poisoned."))?;
        let contains_match =
            |diagnostics: &[PublisherDiagnostic]| diagnostics.iter().any(&predicate);
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

fn recording_reporter() -> (PublisherReporter, Arc<RecordedDiagnostics>) {
    let diagnostics = Arc::new(RecordedDiagnostics::new());
    let recorded_diagnostics = Arc::clone(&diagnostics);
    let reporter: PublisherReporter = Arc::new(move |diagnostic| {
        recorded_diagnostics.record(diagnostic);
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

fn advance_publisher_clock(
    clock: &ControlledClock,
    publisher: &CompletedChatboxPublisher,
    duration: Duration,
) {
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

    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::UnitDroppedOverload {
            unit_id,
            page_count: 2,
        } if unit_id == "unit-a"
    ))?);

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

    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::UnitSendFailed {
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
fn started_unit_is_protected_and_new_unit_is_rejected_without_evicting_others() -> AppResult<()> {
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

    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::UnitRejectedOverload {
            unit_id,
            page_count: 2,
        } if unit_id == "unit-c"
    ))?);

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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::UnitRejectedOverload {
            unit_id,
            page_count: 3,
        } if unit_id == "oversized"
    ))?);

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
        PublisherDiagnostic::UnitExpired {
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
fn active_typing_is_reasserted_before_vrchat_hides_it() -> AppResult<()> {
    let clock = Arc::new(ControlledClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxPacer::with_clock(clock.clone()),
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
        CompletedPublisherEvent::Aborted {
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
        ChatboxPacer::with_clock(clock.clone()),
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
        CompletedPublisherEvent::Aborted {
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
            PublisherDiagnostic::TypingFailed {
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
    let generation = RuntimeGeneration::active();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxPacer::with_clock(clock.clone()),
        generation.clone(),
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_pages: 4,
            max_unstarted_age: Duration::from_secs(30),
        },
    )?;

    submit_handled(
        &publisher,
        CompletedPublisherEvent::Started {
            unit_id: "stopped-before-refresh".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    wait_for_next_typing_reassert(clock.as_ref(), &publisher)?;
    advance_publisher_clock(clock.as_ref(), &publisher, Duration::from_secs(3));

    generation.request_stop(Some(&publisher))?;
    publisher.join()?;
    generation.request_stop(Some(&publisher))?;
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
    let generation = RuntimeGeneration::active();
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxPacer::with_clock(clock.clone()),
        generation.clone(),
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_pages: 4,
            max_unstarted_age: Duration::from_secs(30),
        },
    )?;

    submit_handled(
        &publisher,
        CompletedPublisherEvent::Started {
            unit_id: "typing-stop-race".to_string(),
        },
    )?;
    transport.wait_for_events(1)?;
    advance_to_next_typing_reassert(clock.as_ref(), &publisher)?;
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Typing reassertion did not reach transport."))?;

    let stop_generation = generation.clone();
    let stop_publisher = publisher.clone();
    let (stop_finished_sender, stop_finished_receiver) = mpsc::channel();
    let stop = thread::spawn(move || {
        let result = stop_generation.request_stop(Some(&stop_publisher));
        let _ = stop_finished_sender.send(());
        result
    });

    let stop_entered_deadline = Instant::now() + Duration::from_secs(1);
    while !generation.is_hard_stopped() {
        if Instant::now() >= stop_entered_deadline {
            return Err(AppError::runtime("Stop did not enter request_stop."));
        }
        thread::sleep(Duration::from_millis(1));
    }
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
fn typing_reassertions_do_not_consume_text_pacing_opportunities() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let transport_clock: Arc<dyn Clock> = clock.clone();
    let transport = Arc::new(ScriptedTransport::new(transport_clock, []));
    let text = "中".repeat(811);
    let page_count = paginate_completed(&text)
        .map_err(|error| AppError::runtime(describe_layout_error(error)))?
        .len();
    assert!(page_count >= 6);
    let publisher = CompletedChatboxPublisher::start_with_limits(
        transport.clone(),
        ChatboxPacer::with_clock(clock),
        RuntimeGeneration::active(),
        Arc::new(|_| {}),
        PublisherLimits {
            max_resident_pages: page_count,
            max_unstarted_age: Duration::from_secs(30),
        },
    )?;

    submit_handled(
        &publisher,
        CompletedPublisherEvent::Started {
            unit_id: "paced-around-typing".to_string(),
        },
    )?;
    submit_handled(
        &publisher,
        CompletedPublisherEvent::Completed {
            unit_id: "paced-around-typing".to_string(),
            text,
        },
    )?;

    let events = transport.wait_for_events(page_count + 3)?;
    let text_attempts = events
        .iter()
        .filter_map(|event| match event.event {
            TransportEvent::Text(_) => Some(event.at),
            TransportEvent::Typing(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(text_attempts.len(), page_count);
    assert!(
        text_attempts
            .windows(2)
            .all(|attempts| { attempts[1].duration_since(attempts[0]) == Duration::from_secs(1) })
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event == TransportEvent::Typing(true))
            .count(),
        2
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
    diagnostics.wait_for(
        "a layout-failure diagnostic for layout-failure",
        |diagnostic| {
            matches!(
                diagnostic,
                PublisherDiagnostic::LayoutFailed { unit_id, .. }
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::TypingFailed {
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::PagesDiscardedOnClose {
            reason: PublisherCloseReason::Stop,
            unit_count: 1,
            page_count: 1,
            started_unit_count: 0,
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
    assert!(diagnostics.contains(|diagnostic| matches!(
        diagnostic,
        PublisherDiagnostic::PagesDiscardedOnClose {
            reason: PublisherCloseReason::Stop,
            page_count: 1,
            started_unit_count: 1,
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
    assert!(
        diagnostics.contains(|diagnostic| matches!(
            diagnostic,
            PublisherDiagnostic::WorkerFailed { .. }
        ))?
    );

    Ok(())
}
