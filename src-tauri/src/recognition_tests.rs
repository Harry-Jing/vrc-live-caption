use super::*;
use crate::caption_session::{CaptionLane, CaptionSnapshotV1, CaptionState};
use crate::error::{AppError, AppResult};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

struct EchoFirstFrameDriver;

impl RecognitionDriver for EchoFirstFrameDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;

        let RecognitionDriverInput::Audio(frame) = io.receive(TEST_TIMEOUT)? else {
            return Err(AppError::state(
                "Recognition contract test did not receive its audio frame.",
            ));
        };
        io.emit(RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            generation: io.scope().generation,
            stream_id: io.scope().stream_id.clone(),
            unit_id: format!("frame-{}", frame.sequence),
            started_at_ms: frame.captured_at_ms,
        }))
    }
}

struct WaitForStopDriver;

impl RecognitionDriver for WaitForStopDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        io.wait_until_stopped()
    }
}

struct PendingAndLateSignalDriver {
    pending_sent: SyncSender<()>,
}

struct ReadyThenFailDriver {
    fail: Receiver<()>,
}

impl RecognitionDriver for ReadyThenFailDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        self.fail.recv_timeout(TEST_TIMEOUT).map_err(|error| {
            AppError::state(format!("Terminal driver trigger was not received: {error}"))
        })?;
        Err(AppError::stt_provider(
            crate::error::ProviderFailureClass::Authentication,
            "The recognition provider rejected the configured credential.",
        ))
    }
}

struct ReconnectAfterFirstFrameDriver;

struct AdmissionEpochRaceDriver {
    begin_reconnect: Receiver<()>,
    delivered_sequence: SyncSender<u64>,
}

struct OngoingFloodThenControlDriver {
    begin_flood: Receiver<()>,
    control_result: SyncSender<bool>,
}

impl RecognitionDriver for OngoingFloodThenControlDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        self.begin_flood
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|error| {
                AppError::state(format!("Signal flood trigger was not received: {error}"))
            })?;
        for revision in 1..=128 {
            io.emit_event(RecognitionEvent::Caption(CaptionSnapshotV1 {
                generation: io.scope().generation,
                stream_id: io.scope().stream_id.clone(),
                unit_id: Some("ongoing-unit".to_string()),
                lane: CaptionLane::Source,
                revision,
                text: format!("draft-{revision}"),
                state: CaptionState::Ongoing,
                language: None,
                provider: "test".to_string(),
                model: "test".to_string(),
                unit_started_at_ms: Some(1),
                timestamp_ms: revision,
            }))?;
        }
        let control_sent = io
            .emit(RecognitionSignal::Reconnecting {
                epoch: 9,
                attempt: 1,
                delay_ms: 500,
            })
            .is_ok();
        self.control_result.send(control_sent).map_err(|_| {
            AppError::state("Signal flood test dropped its control-result receiver.")
        })?;
        io.wait_until_stopped()
    }
}

impl RecognitionDriver for AdmissionEpochRaceDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        self.begin_reconnect
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|error| {
                AppError::state(format!("Reconnect race trigger was not received: {error}"))
            })?;
        io.reconnecting(41, 1, Duration::from_millis(1))?;
        io.ready(true)?;

        let RecognitionDriverInput::Audio(frame) = io.receive(TEST_TIMEOUT)? else {
            return Err(AppError::state(
                "Fresh-attempt audio was not received after reconnect.",
            ));
        };
        self.delivered_sequence.send(frame.sequence).map_err(|_| {
            AppError::state("Reconnect race test dropped its delivered-frame receiver.")
        })?;
        io.wait_until_stopped()
    }
}

impl RecognitionDriver for ReconnectAfterFirstFrameDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        let RecognitionDriverInput::Audio(_) = io.receive(TEST_TIMEOUT)? else {
            return Err(AppError::state(
                "Recognition reconnect test did not receive its first audio frame.",
            ));
        };
        io.reconnecting(7, 1, Duration::from_millis(500))?;
        io.ready(true)?;
        io.wait_until_stopped()
    }
}

impl RecognitionDriver for PendingAndLateSignalDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        io.emit(RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            generation: io.scope().generation,
            stream_id: io.scope().stream_id.clone(),
            unit_id: "pending-unit".to_string(),
            started_at_ms: 321,
        }))?;
        self.pending_sent.send(()).map_err(|_| {
            AppError::state("Recognition contract test dropped its synchronization receiver.")
        })?;
        io.wait_until_stopped()?;

        if io
            .emit(RecognitionSignal::Event(RecognitionEvent::UnitStarted {
                generation: io.scope().generation,
                stream_id: io.scope().stream_id.clone(),
                unit_id: "late-unit".to_string(),
                started_at_ms: 999,
            }))
            .is_ok()
        {
            return Err(AppError::state(
                "Recognition accepted a signal after its Stop fence.",
            ));
        }

        Ok(())
    }
}

fn scope() -> RecognitionGenerationScope {
    RecognitionGenerationScope {
        generation: 17,
        stream_id: "recognition-17-1".to_string(),
    }
}

fn frame(sequence: u64) -> OwnedRecognitionAudioFrame {
    OwnedRecognitionAudioFrame {
        sequence,
        captured_at_ms: 123,
        sample_rate_hz: 16_000,
        samples: vec![0.25; 160].into_boxed_slice(),
    }
}

fn short_frame(sequence: u64) -> OwnedRecognitionAudioFrame {
    OwnedRecognitionAudioFrame {
        sequence,
        captured_at_ms: 123,
        sample_rate_hz: 16_000,
        samples: vec![0.25; 80].into_boxed_slice(),
    }
}

fn ongoing_signal(unit_id: &str, revision: u64) -> RecognitionSignal {
    RecognitionSignal::Event(RecognitionEvent::Caption(CaptionSnapshotV1 {
        generation: 17,
        stream_id: "stream-17".to_string(),
        unit_id: Some(unit_id.to_string()),
        lane: CaptionLane::Source,
        revision,
        text: format!("draft-{revision}"),
        state: CaptionState::Ongoing,
        language: None,
        provider: "test".to_string(),
        model: "test".to_string(),
        unit_started_at_ms: Some(1),
        timestamp_ms: revision,
    }))
}

#[test]
fn active_session_accepts_owned_audio_and_emits_generation_scoped_signals() -> AppResult<()> {
    let module =
        RecognitionModule::with_audio_budget(Duration::from_millis(100), 1, EchoFirstFrameDriver)?;
    let mut running = module.start(scope())?;

    let ready = running
        .signals
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| AppError::state(format!("Ready signal was not received: {error}")))?;
    assert_eq!(
        ready,
        RecognitionSignal::Ready {
            generation: 17,
            stream_id: "recognition-17-1".to_string(),
            recovered: false,
        }
    );

    running
        .try_submit(frame(7))
        .map_err(|error| AppError::state(format!("Audio frame was rejected: {error:?}")))?;
    let signal = running
        .signals
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| {
            AppError::state(format!("Recognition signal was not received: {error}"))
        })?;
    assert_eq!(
        signal,
        RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            generation: 17,
            stream_id: "recognition-17-1".to_string(),
            unit_id: "frame-7".to_string(),
            started_at_ms: 123,
        })
    );

    running.stop()
}

#[test]
fn try_submit_fails_closed_when_bounded_ingress_is_full() -> AppResult<()> {
    let module =
        RecognitionModule::with_audio_budget(Duration::from_millis(100), 1, WaitForStopDriver)?;
    let mut running = module.start(scope())?;
    let _ready = running
        .signals
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| AppError::state(format!("Ready signal was not received: {error}")))?;

    assert_eq!(running.try_submit(frame(1)), Ok(()));
    assert_eq!(
        running.try_submit(frame(2)),
        Err(RecognitionSubmitError::Backpressure)
    );
    assert_eq!(
        running.try_submit(frame(3)),
        Err(RecognitionSubmitError::Stopped)
    );

    running.stop()
}

#[test]
fn ingress_budget_is_measured_in_audio_duration_not_capture_callbacks() -> AppResult<()> {
    let module =
        RecognitionModule::with_audio_budget(Duration::from_millis(20), 8, WaitForStopDriver)?;
    let mut running = module.start(scope())?;
    let _ready = running
        .signals
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| AppError::state(format!("Ready signal was not received: {error}")))?;

    for sequence in 1..=4 {
        assert_eq!(running.try_submit(short_frame(sequence)), Ok(()));
    }
    assert_eq!(
        running.try_submit(short_frame(5)),
        Err(RecognitionSubmitError::Backpressure)
    );

    running.stop()
}

#[test]
fn reconnect_pauses_audio_until_capture_shutdown_is_acknowledged() -> AppResult<()> {
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        8,
        ReconnectAfterFirstFrameDriver,
    )?;
    let mut running = module.start(scope())?;
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready {
            recovered: false,
            ..
        })
    ));
    assert_eq!(running.try_submit(frame(1)), Ok(()));

    let reconnect_epoch = match running.signals.recv_timeout(TEST_TIMEOUT) {
        Ok(RecognitionSignal::Reconnecting {
            epoch,
            attempt: 1,
            delay_ms: 500,
        }) => epoch,
        Ok(other) => {
            return Err(AppError::state(format!(
                "Recognition emitted an unexpected reconnect signal: {other:?}"
            )));
        }
        Err(error) => {
            return Err(AppError::state(format!(
                "Recognition reconnect signal was not received: {error}"
            )));
        }
    };
    assert_eq!(
        running.try_submit(frame(2)),
        Err(RecognitionSubmitError::NotReady)
    );
    assert_eq!(running.signals.try_recv(), Err(TryRecvError::Empty));

    assert!(
        running
            .acknowledge_capture_paused(reconnect_epoch.saturating_add(1))
            .is_err()
    );
    assert_eq!(running.signals.try_recv(), Err(TryRecvError::Empty));
    running.acknowledge_capture_paused(reconnect_epoch)?;
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready {
            recovered: true,
            ..
        })
    ));

    running.stop()
}

#[test]
fn audio_admitted_before_reconnect_cannot_cross_into_the_fresh_attempt() -> AppResult<()> {
    let (begin_reconnect, reconnect_trigger) = mpsc::sync_channel(1);
    let (delivered_sequence, delivered) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        8,
        AdmissionEpochRaceDriver {
            begin_reconnect: reconnect_trigger,
            delivered_sequence,
        },
    )?;
    let mut running = module.start(scope())?;
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready {
            recovered: false,
            ..
        })
    ));

    assert!(matches!(
        running.try_submit_with_hook(frame(1), || {
            begin_reconnect.send(()).map_err(|_| {
                AppError::state("Reconnect race driver stopped before its trigger.")
            })?;
            let epoch = match running.signals.recv_timeout(TEST_TIMEOUT) {
                Ok(RecognitionSignal::Reconnecting { epoch, .. }) => epoch,
                signal => {
                    return Err(AppError::state(format!(
                        "Expected reconnect signal during submit race, received {signal:?}."
                    )));
                }
            };
            running.acknowledge_capture_paused(epoch)?;
            if !matches!(
                running.signals.recv_timeout(TEST_TIMEOUT),
                Ok(RecognitionSignal::Ready {
                    recovered: true,
                    ..
                })
            ) {
                return Err(AppError::state(
                    "Fresh Ready was not received during the submit race.",
                ));
            }
            Ok(())
        }),
        Ok(Ok(()))
    ));
    assert_eq!(running.try_submit(frame(2)), Ok(()));
    assert_eq!(
        delivered.recv_timeout(TEST_TIMEOUT),
        Ok(2),
        "the fresh attempt must never consume the retired attempt's frame"
    );

    running.stop()
}

#[test]
fn stop_discards_pending_signals_and_rejects_late_signals() -> AppResult<()> {
    let (pending_sent, pending_received) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        PendingAndLateSignalDriver { pending_sent },
    )?;
    let mut running = module.start(scope())?;
    let _ready = running
        .signals
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| AppError::state(format!("Ready signal was not received: {error}")))?;
    pending_received
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| {
            AppError::state(format!(
                "Pending signal was not queued before Stop: {error}"
            ))
        })?;

    running.stop()?;

    assert_eq!(running.signals.try_recv(), Err(TryRecvError::Disconnected));
    Ok(())
}

#[test]
fn drop_requests_stop_while_the_driver_waits_for_capture_ack() -> AppResult<()> {
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        8,
        ReconnectAfterFirstFrameDriver,
    )?;
    let running = module.start(scope())?;
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready { .. })
    ));
    assert_eq!(running.try_submit(frame(1)), Ok(()));
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Reconnecting { .. })
    ));

    let started_at = Instant::now();
    drop(running);
    assert!(started_at.elapsed() < TEST_TIMEOUT);
    Ok(())
}

#[test]
fn invalid_audio_is_rejected_before_it_consumes_ingress_budget() -> AppResult<()> {
    let module =
        RecognitionModule::with_audio_budget(Duration::from_millis(10), 1, WaitForStopDriver)?;
    let mut running = module.start(scope())?;
    let _ready = running
        .signals
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|error| AppError::state(format!("Ready signal was not received: {error}")))?;

    let mut empty = frame(1);
    empty.samples = Box::new([]);
    assert_eq!(
        running.try_submit(empty),
        Err(RecognitionSubmitError::InvalidAudio)
    );
    let mut zero_rate = frame(2);
    zero_rate.sample_rate_hz = 0;
    assert_eq!(
        running.try_submit(zero_rate),
        Err(RecognitionSubmitError::InvalidAudio)
    );
    assert_eq!(running.try_submit(frame(3)), Ok(()));

    running.stop()
}

#[test]
fn coalesced_ongoing_caption_keeps_its_latest_emission_position() -> AppResult<()> {
    let (signals, received) = recognition_signal_queue();

    assert!(signals.try_send(ongoing_signal("unit-a", 1)).is_ok());
    assert!(
        signals
            .try_send(RecognitionSignal::Event(RecognitionEvent::UnitStarted {
                generation: 17,
                stream_id: "stream-17".to_string(),
                unit_id: "unit-b".to_string(),
                started_at_ms: 2,
            }))
            .is_ok()
    );
    assert!(signals.try_send(ongoing_signal("unit-a", 2)).is_ok());

    assert!(matches!(
        received.try_recv(),
        Ok(RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            unit_id,
            ..
        })) if unit_id == "unit-b"
    ));
    assert!(matches!(
        received.try_recv(),
        Ok(RecognitionSignal::Event(RecognitionEvent::Caption(
            CaptionSnapshotV1 { revision: 2, .. }
        )))
    ));
    assert_eq!(received.try_recv(), Err(TryRecvError::Empty));
    Ok(())
}

#[test]
fn lifecycle_signals_use_reserved_capacity_after_ongoing_queue_saturates() -> AppResult<()> {
    let (signals, _received) = recognition_signal_queue();
    for unit in 0..(RECOGNITION_SIGNAL_QUEUE_CAPACITY - RECOGNITION_SIGNAL_CONTROL_RESERVE) {
        assert!(
            signals
                .try_send(ongoing_signal(&format!("ongoing-{unit}"), 1))
                .is_ok()
        );
    }

    assert!(
        signals
            .try_send(RecognitionSignal::Event(RecognitionEvent::UnitEnded {
                generation: 17,
                stream_id: "stream-17".to_string(),
                unit_id: "durable-unit".to_string(),
                reason: RecognitionEndReason::NoSpeech,
            }))
            .is_ok()
    );
    Ok(())
}

#[test]
fn ongoing_flood_is_coalesced_without_blocking_lifecycle_control() -> AppResult<()> {
    let (begin_flood, flood_trigger) = mpsc::sync_channel(1);
    let (control_result, control_sent) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        OngoingFloodThenControlDriver {
            begin_flood: flood_trigger,
            control_result,
        },
    )?;
    let mut running = module.start(scope())?;
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready { .. })
    ));
    begin_flood
        .send(())
        .map_err(|_| AppError::state("Signal flood driver stopped before its trigger."))?;

    assert_eq!(control_sent.recv_timeout(TEST_TIMEOUT), Ok(true));
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Event(RecognitionEvent::Caption(
            CaptionSnapshotV1 {
                revision: 128,
                state: CaptionState::Ongoing,
                ..
            }
        )))
    ));
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Reconnecting { epoch: 9, .. })
    ));

    running.stop()
}

#[test]
fn terminal_driver_exit_closes_admission_and_preserves_the_structured_error() -> AppResult<()> {
    let (fail, fail_trigger) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        ReadyThenFailDriver { fail: fail_trigger },
    )?;
    let mut running = module.start(scope())?;
    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready { .. })
    ));
    fail.send(())
        .map_err(|_| AppError::state("Terminal driver stopped before its trigger."))?;

    let deadline = Instant::now() + TEST_TIMEOUT;
    while running.is_accepting_audio() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!running.is_accepting_audio());
    assert_eq!(
        running.try_submit(frame(1)),
        Err(RecognitionSubmitError::Stopped)
    );
    let error = match running.stop() {
        Err(error) => error,
        Ok(()) => {
            return Err(AppError::state(
                "Terminal driver error did not survive owner join.",
            ));
        }
    };
    assert_eq!(error.code(), "stt.provider_authentication_failed");
    Ok(())
}
