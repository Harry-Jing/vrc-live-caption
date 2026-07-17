use super::*;
use crate::capability_planner::ResolvedPublicationPolicy;
use crate::caption_session::{
    CaptionActiveUnitV1, CaptionLane, CaptionSessionActiveV1, CaptionSnapshotV1, CaptionState,
};
use crate::chatbox_pacer::Clock;
use std::collections::HashSet;
use std::sync::Condvar;

#[derive(Clone, Debug, PartialEq, Eq)]
enum TransportEvent {
    Text(String),
    Typing(bool),
}

struct ManualClock {
    state: Mutex<ManualClockState>,
    changed: Condvar,
}

struct ManualClockState {
    now: Instant,
    sleep_calls: usize,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            state: Mutex::new(ManualClockState {
                now: Instant::now(),
                sleep_calls: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn advance(&self, duration: Duration) {
        if let Ok(mut state) = self.state.lock() {
            state.now += duration;
            self.changed.notify_all();
        }
    }

    fn wait_for_sleep(&self) -> AppResult<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Manual clock lock was poisoned."))?;
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(1), |state| {
                state.sleep_calls == 0
            })
            .map_err(|_| AppError::state("Manual clock lock was poisoned."))?;
        if timeout.timed_out() && state.sleep_calls == 0 {
            return Err(AppError::runtime(
                "Publisher did not enter a controlled pacing sleep.",
            ));
        }
        Ok(())
    }
}

impl Clock for ManualClock {
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
        state.sleep_calls += 1;
        self.changed.notify_all();
        while state.now < deadline {
            let Ok(next) = self.changed.wait(state) else {
                return;
            };
            state = next;
        }
    }
}

struct RecordingTransport {
    events: Mutex<Vec<TransportEvent>>,
    changed: Condvar,
    failed_text_attempts: HashSet<usize>,
    next_text_attempt: Mutex<usize>,
}

struct PanicOnTypingTransport {
    recording: RecordingTransport,
    panic_on_typing_on: Mutex<bool>,
}

impl PanicOnTypingTransport {
    fn new() -> Self {
        Self {
            recording: RecordingTransport::new([]),
            panic_on_typing_on: Mutex::new(true),
        }
    }
}

impl RecordingTransport {
    fn new(failed_text_attempts: impl IntoIterator<Item = usize>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            changed: Condvar::new(),
            failed_text_attempts: failed_text_attempts.into_iter().collect(),
            next_text_attempt: Mutex::new(1),
        }
    }

    fn text_events(&self) -> AppResult<Vec<String>> {
        self.events
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter_map(|event| match event {
                        TransportEvent::Text(text) => Some(text.clone()),
                        TransportEvent::Typing(_) => None,
                    })
                    .collect()
            })
            .map_err(|_| AppError::state("Recording transport lock was poisoned."))
    }

    fn events(&self) -> AppResult<Vec<TransportEvent>> {
        self.events
            .lock()
            .map(|events| events.clone())
            .map_err(|_| AppError::state("Recording transport lock was poisoned."))
    }

    fn wait_for_typing_attempts(&self, is_typing: bool, count: usize) -> AppResult<()> {
        let events = self
            .events
            .lock()
            .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;
        let (events, timeout) = self
            .changed
            .wait_timeout_while(events, Duration::from_secs(1), |events| {
                events
                    .iter()
                    .filter(|event| **event == TransportEvent::Typing(is_typing))
                    .count()
                    < count
            })
            .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;
        let observed = events
            .iter()
            .filter(|event| **event == TransportEvent::Typing(is_typing))
            .count();
        if timeout.timed_out() && observed < count {
            return Err(AppError::runtime(format!(
                "Expected {count} typing-{is_typing} attempt(s), observed {observed}."
            )));
        }
        Ok(())
    }

    fn wait_for_texts(&self, count: usize) -> AppResult<Vec<String>> {
        let events = self
            .events
            .lock()
            .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;
        let (events, timeout) = self
            .changed
            .wait_timeout_while(events, Duration::from_secs(1), |events| {
                events
                    .iter()
                    .filter(|event| matches!(event, TransportEvent::Text(_)))
                    .count()
                    < count
            })
            .map_err(|_| AppError::state("Recording transport lock was poisoned."))?;
        let texts = events
            .iter()
            .filter_map(|event| match event {
                TransportEvent::Text(text) => Some(text.clone()),
                TransportEvent::Typing(_) => None,
            })
            .collect::<Vec<_>>();
        if timeout.timed_out() && texts.len() < count {
            return Err(AppError::runtime(format!(
                "Expected {count} text attempt(s), observed {}.",
                texts.len()
            )));
        }
        Ok(texts)
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
        let attempt = {
            let mut next = self
                .next_text_attempt
                .lock()
                .map_err(|_| AppError::state("Text-attempt lock was poisoned."))?;
            let attempt = *next;
            *next = next.saturating_add(1);
            attempt
        };
        self.record(TransportEvent::Text(text.to_string()))?;
        if self.failed_text_attempts.contains(&attempt) {
            return Err(AppError::osc_send(
                "recording",
                format!("Scripted text failure {attempt}."),
            ));
        }
        Ok(ChatboxSendReceipt {
            target: "recording".to_string(),
            byte_count: text.len(),
        })
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.record(TransportEvent::Typing(is_typing))
    }
}

impl ChatboxTransport for PanicOnTypingTransport {
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        self.recording.send_text(text)
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.recording.send_typing(is_typing)?;
        let should_panic = if is_typing {
            let mut panic_on_typing_on = self
                .panic_on_typing_on
                .lock()
                .map_err(|_| AppError::state("Panic transport lock was poisoned."))?;
            std::mem::take(&mut *panic_on_typing_on)
        } else {
            false
        };
        if should_panic {
            std::panic::resume_unwind(Box::new("scripted Live typing panic"));
        }
        Ok(())
    }
}

fn reporter() -> LivePublisherReporter {
    Arc::new(|_| {})
}

fn recording_reporter() -> (
    LivePublisherReporter,
    Arc<Mutex<Vec<LivePublisherDiagnostic>>>,
) {
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let reporter_diagnostics = Arc::clone(&diagnostics);
    let reporter = Arc::new(move |diagnostic| {
        if let Ok(mut diagnostics) = reporter_diagnostics.lock() {
            diagnostics.push(diagnostic);
        }
    });
    (reporter, diagnostics)
}

fn active() -> CaptionSessionActiveV1 {
    CaptionSessionActiveV1 {
        generation: 1,
        stream_id: "recognition-1-1".to_string(),
    }
}

fn caption(
    unit_id: Option<&str>,
    revision: u64,
    text: &str,
    state: CaptionState,
) -> CaptionSnapshotV1 {
    CaptionSnapshotV1 {
        generation: 1,
        stream_id: "recognition-1-1".to_string(),
        unit_id: unit_id.map(str::to_string),
        lane: CaptionLane::Source,
        revision,
        text: text.to_string(),
        state,
        language: Some("en".to_string()),
        provider: "fake".to_string(),
        model: "scripted".to_string(),
        unit_started_at_ms: unit_id.map(|_| 100),
        timestamp_ms: 100 + revision,
    }
}

fn snapshot(
    revision: u64,
    active_units: &[&str],
    captions: Vec<CaptionSnapshotV1>,
) -> CaptionSessionSnapshotV1 {
    CaptionSessionSnapshotV1 {
        contract_version: 1,
        snapshot_revision: revision,
        active: Some(active()),
        active_units: active_units
            .iter()
            .map(|unit_id| CaptionActiveUnitV1 {
                unit_id: (*unit_id).to_string(),
                started_at_ms: 100,
            })
            .collect(),
        captions,
    }
}

fn start_publisher(
    clock: Arc<ManualClock>,
    transport: Arc<RecordingTransport>,
    policy: ResolvedPublicationPolicy,
) -> AppResult<(LiveChatboxPublisher, ChatboxPacer)> {
    let pacer = ChatboxPacer::with_clock(clock);
    let publisher = LiveChatboxPublisher::start(
        transport,
        pacer.clone(),
        1,
        RuntimeGeneration::active(),
        policy,
        reporter(),
    )?;
    Ok((publisher, pacer))
}

fn start_unit_publisher(
    clock: Arc<ManualClock>,
    transport: Arc<RecordingTransport>,
) -> AppResult<(LiveChatboxPublisher, ChatboxPacer)> {
    start_unit_publisher_with_window(clock, transport, 1_000)
}

fn start_unit_publisher_with_window(
    clock: Arc<ManualClock>,
    transport: Arc<RecordingTransport>,
    observation_window_ms: u64,
) -> AppResult<(LiveChatboxPublisher, ChatboxPacer)> {
    start_publisher(
        clock,
        transport,
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms,
        },
    )
}

fn start_unitless_publisher(
    clock: Arc<ManualClock>,
    transport: Arc<RecordingTransport>,
    first_non_empty_delay_ms: u64,
) -> AppResult<(LiveChatboxPublisher, ChatboxPacer)> {
    start_publisher(
        clock,
        transport,
        ResolvedPublicationPolicy::LiveUnitless {
            first_non_empty_delay_ms,
        },
    )
}

fn close(publisher: &LiveChatboxPublisher) -> AppResult<()> {
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()
}

fn observe(publisher: &LiveChatboxPublisher, snapshot: &CaptionSessionSnapshotV1) -> AppResult<()> {
    assert_eq!(
        publisher.try_observe(snapshot)?,
        PublisherSubmitOutcome::Handled
    );
    Ok(())
}

#[test]
fn rejects_non_live_resolved_policy() {
    let result = start_publisher(
        Arc::new(ManualClock::new()),
        Arc::new(RecordingTransport::new([])),
        ResolvedPublicationPolicy::Completed,
    );

    assert!(result.is_err());
}

#[test]
fn rejects_zero_live_observation_delays() {
    for policy in [
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 0,
        },
        ResolvedPublicationPolicy::LiveUnitless {
            first_non_empty_delay_ms: 0,
        },
    ] {
        let result = start_publisher(
            Arc::new(ManualClock::new()),
            Arc::new(RecordingTransport::new([])),
            policy,
        );
        assert!(result.is_err());
    }
}

#[test]
fn rejects_a_mismatched_runtime_generation() {
    let result = LiveChatboxPublisher::start(
        Arc::new(RecordingTransport::new([])),
        ChatboxPacer::with_clock(Arc::new(ManualClock::new())),
        2,
        RuntimeGeneration::active(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter(),
    );

    assert!(result.is_err());
}

#[test]
fn active_unit_turns_typing_on_reasserts_at_four_seconds_and_cleans_up_once() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;

    observe(&publisher, &snapshot(1, &["unit-1"], vec![]))?;
    transport.wait_for_typing_attempts(true, 1)?;

    clock.advance(Duration::from_secs(4));
    publisher.shared.wake.notify_all();
    transport.wait_for_typing_attempts(true, 2)?;

    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;
    transport.wait_for_typing_attempts(false, 1)?;

    let typing = transport
        .events()?
        .into_iter()
        .filter(|event| matches!(event, TransportEvent::Typing(_)))
        .collect::<Vec<_>>();
    assert_eq!(
        typing,
        [
            TransportEvent::Typing(true),
            TransportEvent::Typing(true),
            TransportEvent::Typing(false),
        ]
    );
    Ok(())
}

#[test]
fn live_viewport_combines_recent_source_captions_in_time_order() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;
    let mut earlier = caption(
        Some("unit-1"),
        1,
        "earlier context",
        CaptionState::Completed,
    );
    earlier.unit_started_at_ms = Some(100);
    earlier.timestamp_ms = 200;
    let mut current = caption(Some("unit-2"), 1, "current words", CaptionState::Ongoing);
    current.unit_started_at_ms = Some(300);
    current.timestamp_ms = 350;

    observe(
        &publisher,
        &snapshot(
            1,
            &["unit-2"],
            // Aggregate storage is newest-first; rendering must be chronological.
            vec![current, earlier],
        ),
    )?;
    clock.advance(Duration::from_secs(1));
    publisher.shared.wake.notify_all();

    assert_eq!(
        transport.wait_for_texts(1)?,
        ["earlier context current words"]
    );
    close(&publisher)
}

#[test]
fn unit_observation_window_comes_from_the_resolved_policy() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher_with_window(clock.clone(), transport.clone(), 250)?;

    observe(&publisher, &snapshot(1, &["unit-1"], vec![]))?;
    observe(
        &publisher,
        &snapshot(
            2,
            &["unit-1"],
            vec![caption(
                Some("unit-1"),
                1,
                "planner-timed draft",
                CaptionState::Ongoing,
            )],
        ),
    )?;
    clock.advance(Duration::from_millis(250));
    publisher.shared.wake.notify_all();

    assert_eq!(transport.wait_for_texts(1)?, ["planner-timed draft"]);
    close(&publisher)
}

#[test]
fn unit_that_completes_during_observation_publishes_only_completion() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;

    observe(&publisher, &snapshot(1, &["unit-1"], vec![]))?;
    observe(
        &publisher,
        &snapshot(
            2,
            &["unit-1"],
            vec![caption(Some("unit-1"), 1, "draft", CaptionState::Ongoing)],
        ),
    )?;
    clock.advance(Duration::from_millis(999));

    observe(
        &publisher,
        &snapshot(
            3,
            &[],
            vec![caption(
                Some("unit-1"),
                2,
                "completed",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert_eq!(transport.wait_for_texts(1)?, ["completed"]);

    close(&publisher)
}

#[test]
fn overlapping_units_do_not_publish_a_newer_draft_before_its_observation_window() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;

    observe(&publisher, &snapshot(1, &["older"], vec![]))?;
    clock.advance(Duration::from_millis(500));
    observe(
        &publisher,
        &snapshot(
            2,
            &["older", "newer"],
            vec![caption(
                Some("newer"),
                1,
                "newer draft",
                CaptionState::Ongoing,
            )],
        ),
    )?;

    clock.advance(Duration::from_millis(100));
    observe(
        &publisher,
        &snapshot(
            3,
            &["newer"],
            vec![
                caption(Some("older"), 1, "older completed", CaptionState::Completed),
                caption(Some("newer"), 1, "newer draft", CaptionState::Ongoing),
            ],
        ),
    )?;
    assert!(transport.text_events()?.is_empty());

    clock.advance(Duration::from_millis(899));
    publisher.shared.wake.notify_all();
    assert!(transport.text_events()?.is_empty());

    clock.advance(Duration::from_millis(1));
    publisher.shared.wake.notify_all();
    assert_eq!(
        transport.wait_for_texts(1)?,
        ["older completed newer draft"]
    );

    close(&publisher)
}

#[test]
fn unitless_stream_waits_from_first_non_empty_and_sends_latest() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unitless_publisher(clock.clone(), transport.clone(), 250)?;

    observe(
        &publisher,
        &snapshot(1, &[], vec![caption(None, 1, "", CaptionState::Ongoing)]),
    )?;
    clock.advance(Duration::from_secs(2));
    assert!(transport.text_events()?.is_empty());

    observe(
        &publisher,
        &snapshot(
            2,
            &[],
            vec![caption(None, 2, "first", CaptionState::Ongoing)],
        ),
    )?;
    clock.advance(Duration::from_millis(249));
    observe(
        &publisher,
        &snapshot(
            3,
            &[],
            vec![caption(None, 3, "latest", CaptionState::Ongoing)],
        ),
    )?;
    assert!(transport.text_events()?.is_empty());

    clock.advance(Duration::from_millis(1));
    assert_eq!(transport.wait_for_texts(1)?, ["latest"]);

    close(&publisher)
}

#[test]
fn newer_snapshot_replaces_candidate_while_pacer_is_waiting() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, pacer) = start_unit_publisher(clock.clone(), transport.clone())?;
    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| AppError::state("Initial pacing permit was cancelled."))?
        .attempt(|| Ok::<(), AppError>(()))?;

    observe(
        &publisher,
        &snapshot(
            1,
            &[],
            vec![caption(
                Some("unit-1"),
                1,
                "obsolete",
                CaptionState::Completed,
            )],
        ),
    )?;
    clock.wait_for_sleep()?;
    observe(
        &publisher,
        &snapshot(
            2,
            &[],
            vec![caption(
                Some("unit-1"),
                2,
                "newest",
                CaptionState::Completed,
            )],
        ),
    )?;
    clock.advance(Duration::from_secs(1));

    assert_eq!(transport.wait_for_texts(1)?, ["newest"]);
    close(&publisher)
}

#[test]
fn identical_completion_is_not_resent_after_successful_ongoing_view() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;

    observe(&publisher, &snapshot(1, &["unit-1"], vec![]))?;
    observe(
        &publisher,
        &snapshot(
            2,
            &["unit-1"],
            vec![caption(
                Some("unit-1"),
                1,
                "same view",
                CaptionState::Ongoing,
            )],
        ),
    )?;
    clock.advance(Duration::from_secs(1));
    assert_eq!(transport.wait_for_texts(1)?, ["same view"]);

    clock.advance(Duration::from_secs(1));
    observe(
        &publisher,
        &snapshot(
            3,
            &[],
            vec![caption(
                Some("unit-1"),
                2,
                "same view",
                CaptionState::Completed,
            )],
        ),
    )?;
    transport.wait_for_typing_attempts(false, 1)?;
    assert_eq!(transport.text_events()?, ["same view"]);

    close(&publisher)
}

#[test]
fn changed_completion_publishes_one_correction_after_ongoing_view() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;

    observe(&publisher, &snapshot(1, &["unit-1"], vec![]))?;
    observe(
        &publisher,
        &snapshot(
            2,
            &["unit-1"],
            vec![caption(
                Some("unit-1"),
                1,
                "draft view",
                CaptionState::Ongoing,
            )],
        ),
    )?;
    clock.advance(Duration::from_secs(1));
    assert_eq!(transport.wait_for_texts(1)?, ["draft view"]);

    clock.advance(Duration::from_secs(1));
    observe(
        &publisher,
        &snapshot(
            3,
            &[],
            vec![caption(
                Some("unit-1"),
                2,
                "corrected view",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert_eq!(
        transport.wait_for_texts(2)?,
        ["draft view", "corrected view"]
    );

    close(&publisher)
}

#[test]
fn successful_view_is_not_reported_as_a_discarded_draft_on_close() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (reporter, diagnostics) = recording_reporter();
    let publisher = LiveChatboxPublisher::start(
        transport.clone(),
        ChatboxPacer::with_clock(clock),
        1,
        RuntimeGeneration::active(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;

    observe(
        &publisher,
        &snapshot(
            1,
            &[],
            vec![caption(
                Some("unit-1"),
                1,
                "published view",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert_eq!(transport.wait_for_texts(1)?, ["published view"]);
    close(&publisher)?;

    let diagnostics = diagnostics
        .lock()
        .map_err(|_| AppError::state("Live diagnostics lock was poisoned."))?;
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, LivePublisherDiagnostic::ViewPublished { .. }))
    );
    assert!(!diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        LivePublisherDiagnostic::DraftDiscardedOnClose { .. }
    )));
    Ok(())
}

#[test]
fn failed_revision_is_not_retried_until_a_new_revision_arrives() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([1]));
    let (publisher, _) = start_unit_publisher(clock.clone(), transport.clone())?;

    observe(
        &publisher,
        &snapshot(
            1,
            &[],
            vec![caption(
                Some("unit-1"),
                1,
                "attempt one",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert_eq!(transport.wait_for_texts(1)?, ["attempt one"]);
    transport.wait_for_typing_attempts(false, 1)?;
    clock.advance(Duration::from_secs(3));
    assert_eq!(transport.text_events()?, ["attempt one"]);

    observe(
        &publisher,
        &snapshot(
            2,
            &[],
            vec![caption(
                Some("unit-1"),
                2,
                "attempt two",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert_eq!(transport.wait_for_texts(2)?, ["attempt one", "attempt two"]);

    close(&publisher)
}

#[test]
fn close_discards_observed_draft_and_sends_one_typing_off_cleanup() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock, transport.clone())?;

    observe(&publisher, &snapshot(1, &["unit-1"], vec![]))?;
    transport.wait_for_typing_attempts(true, 1)?;
    observe(
        &publisher,
        &snapshot(
            2,
            &["unit-1"],
            vec![caption(
                Some("unit-1"),
                1,
                "must not send",
                CaptionState::Ongoing,
            )],
        ),
    )?;
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;

    assert!(transport.text_events()?.is_empty());
    let typing = transport
        .events()?
        .into_iter()
        .filter(|event| matches!(event, TransportEvent::Typing(_)))
        .collect::<Vec<_>>();
    assert_eq!(
        typing,
        [TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );
    Ok(())
}

#[test]
fn stop_before_the_generation_commit_reports_the_unattempted_draft() -> AppResult<()> {
    let generation = RuntimeGeneration::active();
    let blocker_generation = generation.clone();
    let (gate_held_sender, gate_held_receiver) = std::sync::mpsc::channel();
    let (release_gate_sender, release_gate_receiver) = std::sync::mpsc::channel();
    let blocker = thread::spawn(move || {
        blocker_generation.commit_if_active(|| {
            let _ = gate_held_sender.send(());
            let _ = release_gate_receiver.recv();
        })
    });
    gate_held_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Generation test gate was not acquired."))?;

    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (reporter, diagnostics) = recording_reporter();
    let publisher = LiveChatboxPublisher::start(
        transport.clone(),
        ChatboxPacer::with_clock(clock),
        1,
        generation.clone(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;
    let selected = LiveCandidate {
        identity: LiveCandidateIdentity {
            scope: LiveScope {
                stream_id: "recognition-1-1".to_string(),
                unit_id: Some("unit-1".to_string()),
            },
            revision: 1,
            state: CaptionState::Completed,
        },
        view: "never attempted".to_string(),
        ready_at: publisher.shared.pacer.now(),
    };
    publisher
        .shared
        .state
        .lock()
        .map_err(|_| AppError::state("Live publisher state lock was poisoned."))?
        .candidate = Some(selected.clone());
    let candidate_shared = Arc::clone(&publisher.shared);
    let candidate_attempt =
        thread::spawn(move || process_live_candidate(&candidate_shared, selected));

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let output_gate_held = match publisher.shared.output_gate.try_lock() {
            Err(std::sync::TryLockError::WouldBlock) => true,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(AppError::state("Live output test gate was poisoned."));
            }
            Ok(gate) => {
                drop(gate);
                false
            }
        };
        if output_gate_held {
            break;
        }
        if Instant::now() >= deadline {
            return Err(AppError::runtime(
                "Live candidate did not reach the generation commit boundary.",
            ));
        }
        thread::yield_now();
    }

    let stop_generation = generation.clone();
    let stop = thread::spawn(move || stop_generation.request_stop(None));
    let deadline = Instant::now() + Duration::from_secs(1);
    while !generation.is_hard_stopped() {
        if Instant::now() >= deadline {
            return Err(AppError::runtime(
                "Stop did not establish its hard boundary.",
            ));
        }
        thread::yield_now();
    }

    release_gate_sender
        .send(())
        .map_err(|_| AppError::runtime("Generation test gate could not be released."))?;
    assert!(
        blocker
            .join()
            .map_err(|_| AppError::runtime("Generation test blocker panicked."))??
    );
    stop.join()
        .map_err(|_| AppError::runtime("Stop test thread panicked."))??;
    candidate_attempt
        .join()
        .map_err(|_| AppError::runtime("Live candidate test thread panicked."))??;
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()?;

    assert!(transport.text_events()?.is_empty());
    let diagnostics = diagnostics
        .lock()
        .map_err(|_| AppError::state("Live diagnostics lock was poisoned."))?;
    assert!(diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        LivePublisherDiagnostic::DraftDiscardedOnClose {
            reason: PublisherCloseReason::Stop,
        }
    )));
    Ok(())
}

#[test]
fn ignores_out_of_order_and_other_generation_aggregates() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (publisher, _) = start_unit_publisher(clock, transport.clone())?;

    observe(
        &publisher,
        &snapshot(
            5,
            &[],
            vec![caption(
                Some("unit-1"),
                1,
                "current",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert_eq!(transport.wait_for_texts(1)?, ["current"]);

    observe(
        &publisher,
        &snapshot(
            4,
            &[],
            vec![caption(
                Some("unit-1"),
                2,
                "out of order",
                CaptionState::Completed,
            )],
        ),
    )?;
    let mut other_generation = snapshot(
        6,
        &[],
        vec![caption(
            Some("unit-2"),
            1,
            "other generation",
            CaptionState::Completed,
        )],
    );
    other_generation.active = Some(CaptionSessionActiveV1 {
        generation: 2,
        stream_id: "recognition-2-1".to_string(),
    });
    other_generation.captions[0].generation = 2;
    other_generation.captions[0].stream_id = "recognition-2-1".to_string();
    observe(&publisher, &other_generation)?;

    assert_eq!(transport.text_events()?, ["current"]);
    close(&publisher)
}

#[test]
fn worker_panic_discards_the_draft_cleans_typing_once_and_reports_failure() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(PanicOnTypingTransport::new());
    let (reporter, diagnostics) = recording_reporter();
    let publisher = LiveChatboxPublisher::start(
        transport.clone(),
        ChatboxPacer::with_clock(clock),
        1,
        RuntimeGeneration::active(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;

    observe(
        &publisher,
        &snapshot(
            1,
            &[],
            vec![caption(
                Some("unit-1"),
                1,
                "discard after panic",
                CaptionState::Completed,
            )],
        ),
    )?;
    assert!(publisher.join().is_err());

    assert_eq!(
        transport.recording.events()?,
        [TransportEvent::Typing(true), TransportEvent::Typing(false)]
    );
    let diagnostics = diagnostics
        .lock()
        .map_err(|_| AppError::state("Live diagnostics lock was poisoned."))?;
    assert!(diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        LivePublisherDiagnostic::DraftDiscardedOnClose {
            reason: PublisherCloseReason::RuntimeError,
        }
    )));
    assert!(diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        LivePublisherDiagnostic::WorkerFailed { reason }
            if reason == "Live publisher worker panicked."
    )));
    Ok(())
}

#[test]
fn poisoned_state_still_wakes_the_worker_and_attempts_one_cleanup() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let transport = Arc::new(RecordingTransport::new([]));
    let (reporter, diagnostics) = recording_reporter();
    let publisher = LiveChatboxPublisher::start(
        transport.clone(),
        ChatboxPacer::with_clock(clock),
        1,
        RuntimeGeneration::active(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;
    let shared = Arc::clone(&publisher.shared);
    let poisoner = thread::spawn(move || {
        if let Ok(_state) = shared.state.lock() {
            std::panic::resume_unwind(Box::new("poison Live publisher state"));
        }
    });
    assert!(poisoner.join().is_err());

    assert!(publisher.request_close(PublisherCloseReason::Stop).is_err());
    assert!(publisher.join().is_err());
    assert_eq!(transport.events()?, [TransportEvent::Typing(false)]);
    let diagnostics = diagnostics
        .lock()
        .map_err(|_| AppError::state("Live diagnostics lock was poisoned."))?;
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, LivePublisherDiagnostic::WorkerFailed { .. }))
    );
    Ok(())
}
