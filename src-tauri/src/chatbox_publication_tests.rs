use super::*;
use crate::capability_planner::ResolvedPublicationPolicy;
use crate::caption_session::{
    CAPTION_SESSION_CONTRACT_VERSION, CaptionLane, CaptionSessionActiveV1, CaptionSnapshotV1,
    CaptionState,
};
use crate::chatbox_pacer::{ChatboxPacer, Clock};
use crate::chatbox_publisher::PublisherReporter;
use crate::chatbox_transport::{ChatboxSendReceipt, ChatboxTransport};
use crate::error::AppError;
use crate::live_chatbox_publisher::LivePublisherReporter;
use crate::runtime_generation::RuntimeGeneration;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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

fn completed_reporter() -> PublisherReporter {
    Arc::new(|_| {})
}

fn live_reporter() -> LivePublisherReporter {
    Arc::new(|_| {})
}

fn start_completed() -> AppResult<(RuntimeChatboxPublisher, Receiver<String>)> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let publisher = CompletedChatboxPublisher::start(
        transport,
        ChatboxPacer::default(),
        RuntimeGeneration::active(),
        completed_reporter(),
    )?;
    Ok((RuntimeChatboxPublisher::Completed(publisher), receiver))
}

fn start_live() -> AppResult<(RuntimeChatboxPublisher, Receiver<String>)> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });
    let publisher = LiveChatboxPublisher::start(
        transport,
        ChatboxPacer::default(),
        RuntimeGeneration::active(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        live_reporter(),
    )?;
    Ok((RuntimeChatboxPublisher::Live(publisher), receiver))
}

fn completed_snapshot(revision: u64, text: &str) -> CaptionSessionSnapshotV1 {
    CaptionSessionSnapshotV1 {
        contract_version: CAPTION_SESSION_CONTRACT_VERSION,
        snapshot_revision: revision,
        active: Some(CaptionSessionActiveV1 {
            generation: 1,
            stream_id: "recognition-1-1".to_string(),
        }),
        active_units: Vec::new(),
        captions: vec![CaptionSnapshotV1 {
            generation: 1,
            stream_id: "recognition-1-1".to_string(),
            unit_id: Some("unit-1".to_string()),
            lane: CaptionLane::Source,
            revision,
            text: text.to_string(),
            state: CaptionState::Completed,
            language: Some("en".to_string()),
            provider: "fake".to_string(),
            model: "scripted".to_string(),
            unit_started_at_ms: Some(100),
            timestamp_ms: 100 + revision,
        }],
    }
}

fn wait_for_text(receiver: &Receiver<String>) -> AppResult<String> {
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| AppError::runtime(format!("Publisher did not send text: {error}")))
}

fn assert_no_text(receiver: &Receiver<String>) {
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
}

fn close(publisher: &RuntimeChatboxPublisher) -> AppResult<()> {
    publisher.request_close(PublisherCloseReason::Stop)?;
    publisher.join()
}

#[test]
fn completed_variant_ignores_snapshots_and_delegates_completed_events() -> AppResult<()> {
    let (publisher, receiver) = start_completed()?;

    assert_eq!(
        publisher.observe_snapshot(&completed_snapshot(1, "ignored snapshot"))?,
        PublisherSubmitOutcome::Handled
    );
    assert_no_text(&receiver);

    assert_eq!(
        publisher.try_submit_completed_event(CompletedPublisherEvent::Completed {
            unit_id: "unit-1".to_string(),
            text: "completed event".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "completed event");

    close(&publisher)
}

#[test]
fn live_variant_ignores_completed_events_and_delegates_snapshots() -> AppResult<()> {
    let (publisher, receiver) = start_live()?;

    assert_eq!(
        publisher.try_submit_completed_event(CompletedPublisherEvent::Completed {
            unit_id: "unit-1".to_string(),
            text: "ignored event".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );
    assert_no_text(&receiver);

    assert_eq!(
        publisher.observe_snapshot(&completed_snapshot(1, "live snapshot"))?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "live snapshot");

    close(&publisher)
}

#[test]
fn active_branch_reports_closed_after_facade_shutdown() -> AppResult<()> {
    let (completed, _receiver) = start_completed()?;
    close(&completed)?;
    assert_eq!(
        completed.try_submit_completed_event(CompletedPublisherEvent::Completed {
            unit_id: "unit-1".to_string(),
            text: "too late".to_string(),
        })?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(
        completed.observe_snapshot(&completed_snapshot(1, "inactive input"))?,
        PublisherSubmitOutcome::Handled
    );

    let (live, _receiver) = start_live()?;
    close(&live)?;
    assert_eq!(
        live.observe_snapshot(&completed_snapshot(1, "too late"))?,
        PublisherSubmitOutcome::Closed
    );
    assert_eq!(
        live.try_submit_completed_event(CompletedPublisherEvent::Completed {
            unit_id: "unit-1".to_string(),
            text: "inactive input".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );

    Ok(())
}

#[test]
fn completed_and_live_variants_share_the_actual_attempt_pacing_boundary() -> AppResult<()> {
    let clock = Arc::new(ManualClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(RecordingTransport { texts });

    let completed = RuntimeChatboxPublisher::Completed(CompletedChatboxPublisher::start(
        transport.clone(),
        pacer.clone(),
        RuntimeGeneration::active(),
        completed_reporter(),
    )?);
    assert_eq!(
        completed.try_submit_completed_event(CompletedPublisherEvent::Completed {
            unit_id: "completed-unit".to_string(),
            text: "completed attempt".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );
    assert_eq!(wait_for_text(&receiver)?, "completed attempt");
    close(&completed)?;

    let live = RuntimeChatboxPublisher::Live(LiveChatboxPublisher::start(
        transport,
        pacer,
        RuntimeGeneration::active(),
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        live_reporter(),
    )?);
    assert_eq!(
        live.observe_snapshot(&completed_snapshot(1, "live attempt"))?,
        PublisherSubmitOutcome::Handled
    );
    clock.wait_for_sleep()?;
    assert_no_text(&receiver);

    clock.advance(Duration::from_secs(1));
    assert_eq!(wait_for_text(&receiver)?, "live attempt");
    close(&live)
}
