use super::pacer::{ChatboxPacer, Clock};
use super::transport::{ChatboxSendReceipt, ChatboxTransport};
use super::*;
use crate::caption::{
    ActiveCaptionStream, CAPTION_AGGREGATE_CONTRACT_VERSION, CaptionAggregateChange,
    CaptionAggregateSnapshot, CaptionAggregateUpdate, CaptionLane, CaptionSnapshot, CaptionState,
    OpenSourceUnit,
};
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::config::OscConfig;
use crate::error::AppError;
use crate::events::{DiagnosticUpdate, emit_diagnostic};
use crate::generation_fence::GenerationFence;
use crate::host_resolver::HostResolver;
use rosc::{OscMessage, OscPacket, OscType, decoder};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::Listener;

struct ManualClock {
    state: Mutex<ManualClockState>,
    changed: Condvar,
}

struct ManualClockState {
    now: Instant,
    sleep_calls: usize,
}

struct AdvancingClock {
    now: Mutex<Instant>,
    sleeps: Mutex<Vec<Duration>>,
}

impl AdvancingClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    fn total_sleep(&self) -> Duration {
        self.sleeps
            .lock()
            .map(|sleeps| sleeps.iter().copied().sum())
            .unwrap_or_default()
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
        if let Ok(mut sleeps) = self.sleeps.lock() {
            sleeps.push(duration);
        }
        if let Ok(mut now) = self.now.lock() {
            *now += duration;
        }
    }
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
                "Live publisher did not wait for the shared pacing boundary.",
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
        state.sleep_calls = state.sleep_calls.saturating_add(1);
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
    texts: mpsc::Sender<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum PublicationEvent {
    Text(String),
    Typing(bool),
}

struct TracingTransport {
    events: mpsc::Sender<PublicationEvent>,
}

impl ChatboxTransport for RecordingTransport {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.texts
            .send(text.as_str().to_string())
            .map_err(|_| AppError::state("Recording transport receiver was dropped."))?;
        Ok(ChatboxSendReceipt {
            target: "recording".to_string(),
            byte_count: text.as_str().len(),
        })
    }

    fn send_typing(&self, _is_typing: bool) -> AppResult<()> {
        Ok(())
    }
}

impl ChatboxTransport for TracingTransport {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.events
            .send(PublicationEvent::Text(text.as_str().to_string()))
            .map_err(|_| AppError::state("Tracing transport receiver was dropped."))?;
        Ok(ChatboxSendReceipt {
            target: "tracing".to_string(),
            byte_count: text.as_str().len(),
        })
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.events
            .send(PublicationEvent::Typing(is_typing))
            .map_err(|_| AppError::state("Tracing transport receiver was dropped."))
    }
}

#[test]
fn text_transport_accepts_only_prepared_chatbox_text() {
    fn assert_signature<T: ChatboxTransport>() {
        let send: fn(&T, &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> =
            <T as ChatboxTransport>::send_text;
        let _ = send;
    }

    assert_signature::<RecordingTransport>();
}

#[test]
fn osc_test_message_uses_the_shared_pacer_and_prepared_transport() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let udp_receiver =
        UdpSocket::bind("127.0.0.1:0").map_err(|error| AppError::osc_bind(error.to_string()))?;
    udp_receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|error| AppError::osc_bind(error.to_string()))?;
    let port = udp_receiver
        .local_addr()
        .map_err(|error| AppError::osc_bind(error.to_string()))?
        .port();
    let runtime_text = prepare_single_message("runtime attempt")
        .map_err(|error| AppError::runtime(describe_layout_error(error)))?
        .ok_or_else(|| AppError::state("Runtime test text must not be empty."))?;

    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| AppError::state("Runtime test pacing was cancelled."))?
        .attempt(|| transport.send_text(&runtime_text))?;
    let receipt = send_test_message(
        &OscConfig {
            host: "127.0.0.1".to_string(),
            port,
            enabled: true,
        },
        &pacer,
        &HostResolver::default(),
    )?;

    let mut datagram = [0_u8; 1024];
    let (size, _) = udp_receiver
        .recv_from(&mut datagram)
        .map_err(|error| AppError::osc_send("test receiver", error.to_string()))?;
    let (_, packet) = decoder::decode_udp(&datagram[..size])
        .map_err(|error| AppError::osc_encode(error.to_string()))?;

    assert_eq!(wait_for_text(&receiver)?, "runtime attempt");
    assert_eq!(clock.total_sleep(), Duration::from_secs(1));
    assert_eq!(receipt.target, format!("127.0.0.1:{port}"));
    assert_eq!(
        packet,
        OscPacket::Message(OscMessage {
            addr: "/chatbox/input".to_string(),
            args: vec![
                OscType::String(OSC_TEST_MESSAGE.to_string()),
                OscType::Bool(true),
                OscType::Bool(false),
            ],
        })
    );
    assert!(receipt.byte_count > OSC_TEST_MESSAGE.len());
    Ok(())
}

#[test]
fn osc_test_resolution_failure_does_not_consume_a_text_attempt() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let resolver = HostResolver::with_lookup(|_, _| {
        Err(std::io::Error::other("Scripted OSC resolution failure."))
    });

    let error = match send_test_message(
        &OscConfig {
            host: "unresolved.test".to_string(),
            port: 9000,
            enabled: true,
        },
        &pacer,
        &resolver,
    ) {
        Ok(_) => {
            return Err(AppError::state(
                "A failed hostname lookup unexpectedly sent the OSC test message.",
            ));
        }
        Err(error) => error,
    };

    let (texts, _receiver) = mpsc::channel();
    let transport = RecordingTransport { texts };
    let next_text = prepare_single_message("next attempt")
        .map_err(|layout_error| AppError::runtime(describe_layout_error(layout_error)))?
        .ok_or_else(|| AppError::state("Next test text must not be empty."))?;
    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| AppError::state("Next test pacing was cancelled."))?
        .attempt(|| transport.send_text(&next_text))?;

    assert_eq!(error.code(), "osc.send_failed");
    assert_eq!(clock.total_sleep(), Duration::ZERO);
    Ok(())
}

fn reporter() -> Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> {
    Arc::new(|_| {})
}

fn start_completed() -> AppResult<(ChatboxPublication, Receiver<String>)> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        1,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter(),
    )?;
    Ok((publication, receiver))
}

fn start_live() -> AppResult<(ChatboxPublication, Receiver<String>)> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        1,
        fence.committer(),
        ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter(),
    )?;
    Ok((publication, receiver))
}

fn completed_snapshot(revision: u64, text: &str) -> CaptionAggregateSnapshot {
    completed_snapshot_for(1, revision, "unit-1", text)
}

fn completed_update(
    revision: u64,
    unit_id: &str,
    text: &str,
    snapshot_keeps_caption: bool,
) -> CaptionAggregateUpdate {
    let mut snapshot = completed_snapshot_for(1, revision, unit_id, text);
    let caption = snapshot.captions[0].clone();
    if !snapshot_keeps_caption {
        snapshot.captions.clear();
    }
    CaptionAggregateUpdate {
        snapshot,
        change: CaptionAggregateChange::CaptionAccepted(caption),
    }
}

fn completed_snapshot_for(
    generation: u64,
    revision: u64,
    unit_id: &str,
    text: &str,
) -> CaptionAggregateSnapshot {
    CaptionAggregateSnapshot {
        contract_version: CAPTION_AGGREGATE_CONTRACT_VERSION,
        snapshot_revision: revision,
        active_stream: Some(ActiveCaptionStream {
            generation,
            stream_id: format!("recognition-{generation}-1"),
        }),
        open_source_units: Vec::new(),
        captions: vec![CaptionSnapshot {
            generation,
            stream_id: format!("recognition-{generation}-1"),
            unit_id: Some(unit_id.to_string()),
            lane: CaptionLane::Source,
            revision,
            text: text.to_string(),
            state: CaptionState::Completed,
            language: Some("en".to_string()),
            source_ref: None,
            unit_started_at_ms: Some(100),
            timestamp_ms: 100 + revision,
        }],
        translation_units: Vec::new(),
    }
}

fn open_snapshot(revision: u64, unit_id: Option<&str>) -> CaptionAggregateSnapshot {
    CaptionAggregateSnapshot {
        contract_version: CAPTION_AGGREGATE_CONTRACT_VERSION,
        snapshot_revision: revision,
        active_stream: Some(ActiveCaptionStream {
            generation: 1,
            stream_id: "recognition-1-1".to_string(),
        }),
        open_source_units: unit_id
            .map(|unit_id| {
                vec![OpenSourceUnit {
                    unit_id: unit_id.to_string(),
                    started_at_ms: 100,
                }]
            })
            .unwrap_or_default(),
        captions: Vec::new(),
        translation_units: Vec::new(),
    }
}

fn opened_update(revision: u64, unit_id: &str) -> CaptionAggregateUpdate {
    CaptionAggregateUpdate {
        snapshot: open_snapshot(revision, Some(unit_id)),
        change: CaptionAggregateChange::SourceUnitOpened(OpenSourceUnit {
            unit_id: unit_id.to_string(),
            started_at_ms: 100,
        }),
    }
}

fn aborted_update(revision: u64, unit_id: &str) -> CaptionAggregateUpdate {
    CaptionAggregateUpdate {
        snapshot: open_snapshot(revision, None),
        change: CaptionAggregateChange::SourceUnitAborted {
            unit_id: unit_id.to_string(),
        },
    }
}

#[test]
fn facade_selects_completed_publication_without_exposing_its_worker() -> AppResult<()> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let (diagnostics, diagnostic_receiver) = mpsc::channel();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |diagnostic| {
        let _ = diagnostics.send(diagnostic);
    });
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        1,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter,
    )?;

    assert_eq!(
        publication.try_submit(&completed_update(1, "unit-1", "completed snapshot", true))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "completed snapshot");
    diagnostic_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Facade did not map the Completed diagnostic."))?;

    publication.request_close(PublisherCloseReason::Stop)?;
    publication.join()
}

#[test]
fn completed_publication_uses_exact_changes_across_revision_gaps_and_deduplicates_updates()
-> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::with_clock(clock.clone()),
        1,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter(),
    )?;
    let old_trimmed = completed_update(10, "old-trimmed", "first accepted", false);
    let later = completed_update(25, "later", "second accepted", true);

    assert_eq!(
        publication.try_submit(&old_trimmed)?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "first accepted");
    assert_eq!(
        publication.try_submit(&old_trimmed)?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(
        publication.try_submit(&later)?,
        PublisherSubmitOutcome::Handled
    );
    clock.advance(Duration::from_secs(1));
    assert_eq!(wait_for_text(&receiver)?, "second accepted");

    close(&publication)
}

#[test]
fn completed_publication_derives_started_completed_and_aborted_from_aggregates() -> AppResult<()> {
    let (events, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(TracingTransport { events });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        1,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter(),
    )?;

    assert_eq!(
        publication.try_submit(&opened_update(1, "aborted-unit"))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Completed publication did not derive Started."))?,
        PublicationEvent::Typing(true)
    );

    assert_eq!(
        publication.try_submit(&aborted_update(2, "aborted-unit"))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Completed publication did not derive Aborted."))?,
        PublicationEvent::Typing(false)
    );

    assert_eq!(
        publication.try_submit(&opened_update(3, "completed-unit"))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Completed publication did not restart typing."))?,
        PublicationEvent::Typing(true)
    );
    assert_eq!(
        publication.try_submit(&completed_update(
            4,
            "completed-unit",
            "completed from aggregate",
            true,
        ))?,
        PublisherSubmitOutcome::Handled
    );
    let terminal_events = [
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Completed publication did not resolve typing."))?,
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Completed publication did not publish text."))?,
    ];
    assert!(terminal_events.contains(&PublicationEvent::Typing(false)));
    assert!(terminal_events.contains(&PublicationEvent::Text(
        "completed from aggregate".to_string()
    )));

    close(&publication)
}

#[test]
fn completed_publication_ignores_duplicate_out_of_order_and_prior_generation_history()
-> AppResult<()> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let report_count = Arc::new(AtomicUsize::new(0));
    let counted_reports = report_count.clone();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |_| {
        counted_reports.fetch_add(1, Ordering::SeqCst);
    });
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        1,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter,
    )?;
    assert_eq!(
        publication.try_submit(&completed_update(2, "unit-1", "publish once", true))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "publish once");

    assert_eq!(
        publication.try_submit(&completed_update(2, "unit-1", "duplicate", true))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(
        publication.try_submit(&completed_update(1, "unit-1", "out of order", true))?,
        PublisherSubmitOutcome::Handled
    );
    close(&publication)?;
    assert_no_text(&receiver);
    assert_eq!(report_count.load(Ordering::SeqCst), 1);

    let (texts, prior_history_receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let prior_report_count = Arc::new(AtomicUsize::new(0));
    let counted_prior_reports = prior_report_count.clone();
    let prior_reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |_| {
        counted_prior_reports.fetch_add(1, Ordering::SeqCst);
    });
    let current = ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        2,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        prior_reporter,
    )?;
    let mut prior_snapshot = completed_snapshot(3, "prior generation");
    prior_snapshot.active_stream = Some(ActiveCaptionStream {
        generation: 2,
        stream_id: "recognition-2-1".to_string(),
    });
    let with_prior_history = CaptionAggregateUpdate {
        change: CaptionAggregateChange::CaptionAccepted(prior_snapshot.captions[0].clone()),
        snapshot: prior_snapshot,
    };
    assert_eq!(
        current.try_submit(&with_prior_history)?,
        PublisherSubmitOutcome::Handled
    );
    close(&current)?;
    assert_no_text(&prior_history_receiver);
    assert_eq!(prior_report_count.load(Ordering::SeqCst), 0);
    Ok(())
}

fn wait_for_text(receiver: &Receiver<String>) -> AppResult<String> {
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| AppError::runtime(format!("Publisher did not send text: {error}")))
}

fn assert_no_text(receiver: &Receiver<String>) {
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
}

fn close(publisher: &ChatboxPublication) -> AppResult<()> {
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()
}

fn start_for_closed_diagnostic(
    timing: ResolvedPublicationTiming,
    reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
) -> AppResult<ChatboxPublication> {
    let (texts, _receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    ChatboxPublication::start_with_transport(
        transport,
        ChatboxPacer::default(),
        1,
        fence.committer(),
        timing,
        reporter,
    )
}

fn receive_diagnostic_code(receiver: &Receiver<String>) -> AppResult<String> {
    let payload = receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
        AppError::runtime("Closed Chatbox submission did not emit its stable diagnostic.")
    })?;
    let event: serde_json::Value = serde_json::from_str(&payload).map_err(|error| {
        AppError::runtime(format!(
            "Closed Chatbox diagnostic was invalid JSON: {error}"
        ))
    })?;
    Ok(event["code"].as_str().unwrap_or_default().to_string())
}

#[test]
fn generation_stop_cutoff_keeps_stop_diagnostics_before_publication_shutdown() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostics, diagnostic_receiver) = mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostics.send(event.payload().to_string());
    });
    let reporter_app = app.handle().clone();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |diagnostic| {
        emit_diagnostic(&reporter_app, diagnostic);
    });
    let (texts, _receiver) = mpsc::channel();
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport(
        Arc::new(RecordingTransport { texts }),
        ChatboxPacer::default(),
        1,
        fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter,
    )?;

    // Runtime establishes the generation cutoff before the potentially
    // blocking publication shutdown call records its own close reason.
    fence.request_stop();
    let outcome = publication.try_submit(&completed_update(
        1,
        "after-stop-cutoff",
        "must not publish",
        true,
    ))?;
    let diagnostic_code = receive_diagnostic_code(&diagnostic_receiver)?;
    publication.request_close(PublisherCloseReason::Stop)?;
    publication.join()?;

    assert_eq!(outcome, PublisherSubmitOutcome::Closed);
    assert_eq!(diagnostic_code, "osc.send_skipped_on_stop");
    Ok(())
}

#[test]
fn closed_facade_restores_policy_specific_diagnostic_codes() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostics, diagnostic_receiver) = mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostics.send(event.payload().to_string());
    });
    let reporter_app = app.handle().clone();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |diagnostic| {
        emit_diagnostic(&reporter_app, diagnostic);
    });

    let completed_stop =
        start_for_closed_diagnostic(ResolvedPublicationTiming::Completed, reporter.clone())?;
    completed_stop.request_close(PublisherCloseReason::Stop)?;
    completed_stop.join()?;
    assert_eq!(
        completed_stop.try_submit(&completed_update(1, "stop", "too late", true))?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(
        receive_diagnostic_code(&diagnostic_receiver)?,
        "osc.send_skipped_on_stop"
    );

    let completed_error =
        start_for_closed_diagnostic(ResolvedPublicationTiming::Completed, reporter.clone())?;
    completed_error.request_close(PublisherCloseReason::RuntimeError)?;
    completed_error.join()?;
    assert_eq!(
        completed_error.try_submit(&completed_update(1, "error", "too late", true))?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(
        receive_diagnostic_code(&diagnostic_receiver)?,
        "osc.completed_unit_discarded_after_close"
    );

    let live_error = start_for_closed_diagnostic(
        ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;
    live_error.request_close(PublisherCloseReason::RuntimeError)?;
    live_error.join()?;
    assert_eq!(
        live_error.try_submit(&completed_update(1, "live-error", "too late", true))?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(
        receive_diagnostic_code(&diagnostic_receiver)?,
        "osc.live_snapshot_discarded_after_close"
    );

    Ok(())
}

#[test]
fn closed_completed_source_activity_updates_remain_silent() -> AppResult<()> {
    let report_count = Arc::new(AtomicUsize::new(0));
    let counted_reports = report_count.clone();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |_| {
        counted_reports.fetch_add(1, Ordering::SeqCst);
    });
    let publication = start_for_closed_diagnostic(ResolvedPublicationTiming::Completed, reporter)?;
    publication.request_close(PublisherCloseReason::RuntimeError)?;
    publication.join()?;

    assert_eq!(
        publication.try_submit(&opened_update(1, "opened-after-close"))?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(
        publication.try_submit(&aborted_update(2, "aborted-after-close"))?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(report_count.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn facade_selects_live_publication_without_exposing_its_worker() -> AppResult<()> {
    let (publisher, receiver) = start_live()?;

    assert_eq!(
        publisher.try_submit(&completed_update(1, "unit-1", "live snapshot", true))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "live snapshot");

    close(&publisher)
}

#[test]
fn completed_facade_sends_the_centrally_prepared_control_policy() -> AppResult<()> {
    let (publication, receiver) = start_completed()?;

    assert_eq!(
        publication.try_submit(&completed_update(
            1,
            "unit-1",
            "one\rtwo\r\nthree\u{0085}four\u{000C}five",
            true,
        ))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "one two\r\nthree four five");

    close(&publication)
}

#[test]
fn live_facade_preserves_edge_separators_and_prepared_spaces() -> AppResult<()> {
    let (publication, receiver) = start_live()?;
    let source = "\r\n\n\u{000B}\u{2028}\u{2029}\rnewest\u{0085}\u{000C}";
    let expected = "\r\n\n\u{000B}\u{2028}\u{2029} newest  ";

    assert_eq!(
        publication.try_submit(&completed_update(1, "unit-1", source, true))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, expected);

    close(&publication)
}

#[test]
fn active_publication_reports_closed_after_facade_shutdown() -> AppResult<()> {
    let (completed, _receiver) = start_completed()?;
    close(&completed)?;
    assert_eq!(
        completed.try_submit(&completed_update(1, "unit-1", "too late", true))?,
        PublisherSubmitOutcome::Closed
    );

    let (live, _receiver) = start_live()?;
    close(&live)?;
    assert_eq!(
        live.try_submit(&completed_update(1, "unit-1", "too late", true))?,
        PublisherSubmitOutcome::Closed
    );

    Ok(())
}

#[test]
fn completed_and_live_publications_share_the_actual_attempt_pacing_boundary() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });

    let completed_fence = GenerationFence::new();
    let completed = ChatboxPublication::start_with_transport(
        transport.clone(),
        pacer.clone(),
        1,
        completed_fence.committer(),
        ResolvedPublicationTiming::Completed,
        reporter(),
    )?;
    assert_eq!(
        completed.try_submit(&completed_update(1, "unit-1", "completed attempt", true))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "completed attempt");
    close(&completed)?;

    let live_fence = GenerationFence::new();
    let live = ChatboxPublication::start_with_transport(
        transport,
        pacer,
        1,
        live_fence.committer(),
        ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter(),
    )?;
    assert_eq!(
        live.try_submit(&completed_update(1, "unit-1", "live attempt", true))?,
        PublisherSubmitOutcome::Handled
    );
    clock.wait_for_sleep()?;
    assert_no_text(&receiver);

    clock.advance(Duration::from_secs(1));
    assert_eq!(wait_for_text(&receiver)?, "live attempt");
    close(&live)
}
