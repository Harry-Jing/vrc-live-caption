use super::pacer::{ChatboxPacer, Clock};
use super::transport::{ChatboxSendReceipt, ChatboxTransport};
use super::*;
use crate::caption::{
    ActiveCaptionStream, CAPTION_AGGREGATE_CONTRACT_VERSION, CaptionAggregateChange,
    CaptionAggregateSnapshot, CaptionAggregateStore, CaptionAggregateUpdate, CaptionLane,
    CaptionSnapshot, CaptionState, OpenSourceUnit, SourceSnapshotRef, TranslationFailureReason,
};
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::config::ContentSelection;
use crate::error::AppError;
use crate::events::{DiagnosticUpdate, emit_diagnostic};
use crate::generation_fence::GenerationFence;
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
        self.wait_for_sleep_calls(1)
    }

    fn wait_for_sleep_calls(&self, expected: usize) -> AppResult<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Manual clock lock was poisoned."))?;
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(1), |state| {
                state.sleep_calls < expected
            })
            .map_err(|_| AppError::state("Manual clock lock was poisoned."))?;
        if timeout.timed_out() && state.sleep_calls < expected {
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
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        self.texts
            .send(text.to_string())
            .map_err(|_| AppError::state("Recording transport receiver was dropped."))?;
        Ok(ChatboxSendReceipt {
            target: "recording".to_string(),
            byte_count: text.len(),
        })
    }

    fn send_typing(&self, _is_typing: bool) -> AppResult<()> {
        Ok(())
    }
}

impl ChatboxTransport for TracingTransport {
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        self.events
            .send(PublicationEvent::Text(text.to_string()))
            .map_err(|_| AppError::state("Tracing transport receiver was dropped."))?;
        Ok(ChatboxSendReceipt {
            target: "tracing".to_string(),
            byte_count: text.len(),
        })
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.events
            .send(PublicationEvent::Typing(is_typing))
            .map_err(|_| AppError::state("Tracing transport receiver was dropped."))
    }
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

fn start_completed_with_content(
    content: ContentSelection,
    pacer: ChatboxPacer,
) -> AppResult<(ChatboxPublication, Receiver<String>)> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport_for_content(
        transport,
        pacer,
        1,
        "recognition-1-1".to_string(),
        fence.committer(),
        ChatboxPublicationPolicy::new(ResolvedPublicationTiming::Completed, content),
        reporter(),
    )?;
    Ok((publication, receiver))
}

fn start_tracing_completed_with_content(
    content: ContentSelection,
) -> AppResult<(ChatboxPublication, Receiver<PublicationEvent>)> {
    let (events, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(TracingTransport { events });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport_for_content(
        transport,
        ChatboxPacer::default(),
        1,
        "recognition-1-1".to_string(),
        fence.committer(),
        ChatboxPublicationPolicy::new(ResolvedPublicationTiming::Completed, content),
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

fn completed_source_caption(unit_id: &str, text: &str, started_at_ms: u64) -> CaptionSnapshot {
    CaptionSnapshot {
        generation: 1,
        stream_id: "recognition-1-1".to_string(),
        unit_id: Some(unit_id.to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: text.to_string(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(started_at_ms),
        timestamp_ms: started_at_ms.saturating_add(1),
    }
}

fn accepted_update(
    store: &CaptionAggregateStore,
    update: Option<CaptionAggregateUpdate>,
) -> AppResult<CaptionAggregateUpdate> {
    update.ok_or_else(|| {
        let snapshot_revision = store.snapshot().map(|snapshot| snapshot.snapshot_revision);
        AppError::state(format!(
            "Caption aggregate did not accept test update at revision {snapshot_revision:?}."
        ))
    })
}

fn begin_translation_unit(
    store: &CaptionAggregateStore,
    publication: &ChatboxPublication,
    unit_id: &str,
    source: &str,
    started_at_ms: u64,
) -> AppResult<crate::caption::ReservedCompletedSource> {
    let opened = accepted_update(
        store,
        store.start_unit(1, "recognition-1-1", unit_id.to_string(), started_at_ms)?,
    )?;
    let _ = publication.try_submit(&opened)?;
    let (source_update, reservation) = store
        .accept_completed_source_for_translation(completed_source_caption(
            unit_id,
            source,
            started_at_ms,
        ))?
        .ok_or_else(|| AppError::state("Caption aggregate did not reserve test Source."))?;
    let _ = publication.try_submit(&source_update)?;
    Ok(reservation)
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
fn translation_only_preserves_source_admission_order_and_omits_failed_units() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let (publication, receiver) = start_completed_with_content(
        ContentSelection::TranslationOnly,
        ChatboxPacer::with_clock(clock.clone()),
    )?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let first = begin_translation_unit(&store, &publication, "first", "private first", 100)?;
    let second = begin_translation_unit(&store, &publication, "second", "private second", 200)?;

    let second_update = second
        .complete_translation("第二".to_string(), "zh-Hans".to_string(), 202)?
        .ok_or_else(|| AppError::state("Second test Translation was not accepted."))?;
    let _ = publication.try_submit(&second_update)?;
    assert_no_text(&receiver);

    let first_update = first
        .fail_translation(TranslationFailureReason::ProviderUnavailable)?
        .ok_or_else(|| AppError::state("First test Translation failure was not accepted."))?;
    let _ = publication.try_submit(&first_update)?;
    assert_eq!(wait_for_text(&receiver)?, "第二");
    assert_no_text(&receiver);

    close(&publication)
}

#[test]
fn bilingual_uses_the_exact_pair_and_continues_after_a_failure() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let (publication, receiver) = start_completed_with_content(
        ContentSelection::Bilingual,
        ChatboxPacer::with_clock(clock.clone()),
    )?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let first = begin_translation_unit(&store, &publication, "first", "source one", 100)?;
    let second = begin_translation_unit(&store, &publication, "second", "source two", 200)?;

    let second_update = second
        .complete_translation("译文二".to_string(), "zh-Hans".to_string(), 202)?
        .ok_or_else(|| AppError::state("Second test Translation was not accepted."))?;
    let _ = publication.try_submit(&second_update)?;
    assert_no_text(&receiver);

    let first_update = first
        .fail_translation(TranslationFailureReason::DeadlineExceeded)?
        .ok_or_else(|| AppError::state("First test Translation failure was not accepted."))?;
    let _ = publication.try_submit(&first_update)?;
    assert_eq!(wait_for_text(&receiver)?, "source one");
    clock.wait_for_sleep()?;
    clock.advance(Duration::from_secs(1));
    assert_eq!(wait_for_text(&receiver)?, "source two\n译文二");

    close(&publication)
}

#[test]
fn bilingual_publication_uses_every_unequal_layout_page_without_repairing_the_pair() -> AppResult<()>
{
    let clock = Arc::new(ManualClock::new());
    let (publication, receiver) = start_completed_with_content(
        ContentSelection::Bilingual,
        ChatboxPacer::with_clock(clock.clone()),
    )?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let source = "source lane ".repeat(80);
    let translation = "短译文";
    let expected = super::layout::paginate_bilingual_completed(&source, translation)
        .map_err(|error| AppError::state(format!("Test bilingual layout failed: {error:?}")))?
        .into_iter()
        .map(|page| page.rendered_text())
        .collect::<Vec<_>>();
    assert!(expected.len() > 1);
    let reservation = begin_translation_unit(&store, &publication, "unequal", &source, 100)?;
    let update = reservation
        .complete_translation(translation.to_string(), "zh-Hans".to_string(), 102)?
        .ok_or_else(|| AppError::state("Unequal test Translation was not accepted."))?;
    let _ = publication.try_submit(&update)?;

    for (index, expected_page) in expected.iter().enumerate() {
        if index > 0 {
            clock.wait_for_sleep_calls(index)?;
            clock.advance(Duration::from_secs(1));
        }
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| AppError::runtime(format!("Bilingual page {index} was not sent.")))?,
            *expected_page
        );
    }
    assert_no_text(&receiver);

    close(&publication)
}

#[test]
fn every_terminal_translation_failure_releases_translation_only_order_without_source_fallback()
-> AppResult<()> {
    let (publication, receiver) =
        start_completed_with_content(ContentSelection::TranslationOnly, ChatboxPacer::default())?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let reasons = [
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

    for (index, reason) in reasons.into_iter().enumerate() {
        let unit_id = format!("failed-{index}");
        let reservation = begin_translation_unit(
            &store,
            &publication,
            &unit_id,
            &format!("private source {index}"),
            100 + index as u64,
        )?;
        let update = reservation
            .fail_translation(reason)?
            .ok_or_else(|| AppError::state("Test Translation failure was not accepted."))?;
        let _ = publication.try_submit(&update)?;
    }

    assert_no_text(&receiver);
    close(&publication)
}

#[test]
fn aborted_open_source_releases_a_later_ready_translation() -> AppResult<()> {
    let (publication, receiver) =
        start_completed_with_content(ContentSelection::TranslationOnly, ChatboxPacer::default())?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let first_open = accepted_update(
        &store,
        store.start_unit(1, "recognition-1-1", "first".to_string(), 100)?,
    )?;
    let _ = publication.try_submit(&first_open)?;
    let second = begin_translation_unit(&store, &publication, "second", "private", 200)?;
    let second_update = second
        .complete_translation("ready".to_string(), "en".to_string(), 202)?
        .ok_or_else(|| AppError::state("Second test Translation was not accepted."))?;
    let _ = publication.try_submit(&second_update)?;
    assert_no_text(&receiver);

    let first_abort = accepted_update(
        &store,
        store.abort_source_unit(1, "recognition-1-1", "first")?,
    )?;
    let _ = publication.try_submit(&first_abort)?;
    assert_eq!(wait_for_text(&receiver)?, "ready");

    close(&publication)
}

#[test]
fn completed_translation_pending_does_not_extend_source_typing_activity() -> AppResult<()> {
    let (publication, receiver) =
        start_tracing_completed_with_content(ContentSelection::TranslationOnly)?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let opened = accepted_update(
        &store,
        store.start_unit(1, "recognition-1-1", "pending".to_string(), 100)?,
    )?;
    let _ = publication.try_submit(&opened)?;

    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Source typing-on was not published."))?,
        PublicationEvent::Typing(true)
    );
    let (source_update, _reservation) = store
        .accept_completed_source_for_translation(completed_source_caption(
            "pending", "private", 100,
        ))?
        .ok_or_else(|| AppError::state("Caption aggregate did not reserve test Source."))?;
    let _ = publication.try_submit(&source_update)?;
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Source typing-off was not published."))?,
        PublicationEvent::Typing(false)
    );
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));

    close(&publication)
}

#[test]
fn mismatched_translation_caption_cannot_release_an_exact_source_slot() -> AppResult<()> {
    let (publication, receiver) =
        start_completed_with_content(ContentSelection::TranslationOnly, ChatboxPacer::default())?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let _reservation = begin_translation_unit(&store, &publication, "exact", "private", 100)?;
    let pending = store.snapshot()?;
    let source = pending
        .captions
        .iter()
        .find(|caption| caption.unit_id.as_deref() == Some("exact"))
        .cloned()
        .ok_or_else(|| AppError::state("Test Source snapshot was missing."))?;
    let source_ref = SourceSnapshotRef {
        generation: source.generation,
        stream_id: source.stream_id.clone(),
        unit_id: "exact".to_string(),
        revision: source.revision,
    };
    let translation = |unit_id: &str, text: &str| CaptionSnapshot {
        generation: source.generation,
        stream_id: source.stream_id.clone(),
        unit_id: Some(unit_id.to_string()),
        lane: CaptionLane::Translation,
        revision: 1,
        text: text.to_string(),
        state: CaptionState::Completed,
        language: Some("zh-Hans".to_string()),
        source_ref: Some(source_ref.clone()),
        unit_started_at_ms: source.unit_started_at_ms,
        timestamp_ms: 102,
    };
    let mut mismatched_snapshot = pending.clone();
    mismatched_snapshot.snapshot_revision += 1;
    let mismatched = CaptionAggregateUpdate {
        snapshot: mismatched_snapshot,
        change: CaptionAggregateChange::CaptionAccepted(translation("wrong", "错误")),
    };
    let _ = publication.try_submit(&mismatched)?;
    assert_no_text(&receiver);

    let mut exact_snapshot = pending;
    exact_snapshot.snapshot_revision += 2;
    let exact = CaptionAggregateUpdate {
        snapshot: exact_snapshot,
        change: CaptionAggregateChange::CaptionAccepted(translation("exact", "正确")),
    };
    let _ = publication.try_submit(&exact)?;
    assert_eq!(wait_for_text(&receiver)?, "正确");

    close(&publication)
}

#[test]
fn ready_translation_backlog_uses_the_existing_bounded_page_queue() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let (publication, receiver) = start_completed_with_content(
        ContentSelection::TranslationOnly,
        ChatboxPacer::with_clock(clock.clone()),
    )?;
    let store = CaptionAggregateStore::default();
    store.begin_generation(1)?;
    let first = begin_translation_unit(&store, &publication, "first", "private first", 100)?;

    for index in 0_u64..40 {
        let unit_id = format!("later-{index}");
        let reservation =
            begin_translation_unit(&store, &publication, &unit_id, "private later", 200 + index)?;
        let update = reservation
            .complete_translation(format!("translated-{index}"), "en".to_string(), 300 + index)?
            .ok_or_else(|| AppError::state("Later test Translation was not accepted."))?;
        let _ = publication.try_submit(&update)?;
    }
    assert_no_text(&receiver);

    let first_update = first
        .complete_translation("translated-first".to_string(), "en".to_string(), 500)?
        .ok_or_else(|| AppError::state("First test Translation was not accepted."))?;
    let _ = publication.try_submit(&first_update)?;
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Bounded backlog did not release the first unit."))?,
        "translated-first"
    );
    for (pacing_wait, index) in (9_u64..38).enumerate() {
        clock.wait_for_sleep_calls(pacing_wait + 1)?;
        clock.advance(Duration::from_secs(1));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
                AppError::runtime(format!("Bounded backlog stopped before unit {index}."))
            })?,
            format!("translated-{index}")
        );
    }
    clock.wait_for_sleep_calls(30)?;
    clock.advance(Duration::from_secs(1));
    assert_no_text(&receiver);

    close(&publication)
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
