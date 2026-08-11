use super::*;
use crate::caption::{CaptionAggregateSnapshot, CaptionAggregateStore};
use crate::recognition::{
    RecognitionDriver, RecognitionDriverIo, RecognitionEvent, RecognitionGenerationScope,
    RecognitionModule,
};
use crate::runtime_control::RuntimeControlStore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;
use tauri::Listener;

struct ReconnectAfterCaptureOpensDriver {
    capture_opened: mpsc::Receiver<()>,
    pause_acknowledged: mpsc::SyncSender<()>,
}

struct TerminalAfterCaptureOpensDriver {
    capture_opened: mpsc::Receiver<()>,
}

struct DropTrackedRecognitionDriver {
    dropped: Arc<AtomicBool>,
}

impl Drop for DropTrackedRecognitionDriver {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl RecognitionDriver for DropTrackedRecognitionDriver {
    fn run(self: Box<Self>, _io: RecognitionDriverIo) -> AppResult<()> {
        Err(AppError::state(
            "Drop-tracked Recognition Driver unexpectedly started.",
        ))
    }
}

impl RecognitionDriver for TerminalAfterCaptureOpensDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        self.capture_opened
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| {
                AppError::state(format!("Runtime test capture did not open: {error}"))
            })?;
        io.emit_event(RecognitionEvent::UnitStarted {
            generation: io.scope().generation,
            stream_id: io.scope().stream_id.clone(),
            unit_id: "terminal-active-unit".to_string(),
            started_at_ms: 321,
        })?;
        Err(AppError::recognition_provider(
            crate::error::ProviderFailureClass::Authentication,
            "The recognition provider rejected the configured credential.",
        ))
    }
}

impl RecognitionDriver for ReconnectAfterCaptureOpensDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        self.capture_opened
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| {
                AppError::state(format!("Runtime test capture did not open: {error}"))
            })?;
        io.reconnecting(7, 1, Duration::from_millis(10))?;
        self.pause_acknowledged
            .send(())
            .map_err(|_| AppError::state("Runtime test dropped its capture-pause receiver."))?;
        io.wait_until_stopped()
    }
}

struct DropAwareRecognitionCapture {
    dropped: Arc<AtomicBool>,
}

impl RecognitionCapture for DropAwareRecognitionCapture {
    fn sample_rate(&self) -> u32 {
        16_000
    }

    fn receive(&self, _timeout: Duration) -> AppResult<Option<Vec<f32>>> {
        Ok(None)
    }
}

impl Drop for DropAwareRecognitionCapture {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn runtime_execution_owns_the_selected_recognition_module() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let generation =
        RuntimeGeneration::activate(app.handle(), 1, CaptionAggregateStore::default())?;
    let driver_dropped = Arc::new(AtomicBool::new(false));
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        DropTrackedRecognitionDriver {
            dropped: Arc::clone(&driver_dropped),
        },
    )?;

    let execution = RuntimeExecution::new(
        app.handle().clone(),
        AudioConfig::default(),
        module,
        None,
        generation,
        control.status_recorder(),
    );
    drop(execution);

    assert!(driver_dropped.load(Ordering::SeqCst));
    Ok(())
}

#[test]
fn coordinator_drops_capture_before_acknowledging_reconnect() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate)?;
    let (capture_opened, opened) = mpsc::sync_channel(1);
    let (pause_acknowledged, acknowledged) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        ReconnectAfterCaptureOpensDriver {
            capture_opened: opened,
            pause_acknowledged,
        },
    )?;
    let mut recognition = module.start(RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let capture_dropped = Arc::new(AtomicBool::new(false));
    let opened_capture_dropped = Arc::clone(&capture_dropped);
    let open_capture = move |_config: &AudioConfig| {
        capture_opened.send(()).map_err(|_| {
            AppError::state("Runtime test recognition owner stopped before capture opened.")
        })?;
        Ok(Box::new(DropAwareRecognitionCapture {
            dropped: Arc::clone(&opened_capture_dropped),
        }) as Box<dyn RecognitionCapture>)
    };

    let stop_generation = generation.clone();
    let observed_capture_drop = Arc::clone(&capture_dropped);
    let stopper = thread::spawn(move || -> AppResult<()> {
        acknowledged
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| {
                AppError::state(format!("Reconnect pause was not acknowledged: {error}"))
            })?;
        if !observed_capture_drop.load(Ordering::SeqCst) {
            return Err(AppError::state(
                "Reconnect was acknowledged before microphone capture was dropped.",
            ));
        }
        stop_generation.request_stop(None)
    });

    coordinate_running_recognition_with_capture(
        app.handle(),
        &AudioConfig::default(),
        None,
        &generation,
        &mut recognition,
        &status_recorder,
        &open_capture,
    )?;
    stopper
        .join()
        .map_err(|_| AppError::runtime("Runtime test stopper thread panicked."))??;
    recognition.stop()?;
    assert!(capture_dropped.load(Ordering::SeqCst));
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Reconnecting
    );
    Ok(())
}

#[test]
fn coordinator_drops_capture_and_preserves_terminal_owner_error() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
    let capture_dropped = Arc::new(AtomicBool::new(false));
    let observed_capture_drop = Arc::clone(&capture_dropped);
    let (aggregate_closed_sender, aggregate_closed_receiver) = mpsc::channel();
    app.listen("caption-aggregate-changed", move |event| {
        let Ok(snapshot) = serde_json::from_str::<CaptionAggregateSnapshot>(event.payload()) else {
            return;
        };
        if snapshot.open_source_units.is_empty() {
            let _ = aggregate_closed_sender.send(observed_capture_drop.load(Ordering::SeqCst));
        }
    });
    let (capture_opened, opened) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        TerminalAfterCaptureOpensDriver {
            capture_opened: opened,
        },
    )?;
    let mut recognition = module.start(RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let opened_capture_dropped = Arc::clone(&capture_dropped);
    let open_capture = move |_config: &AudioConfig| {
        capture_opened.send(()).map_err(|_| {
            AppError::state("Runtime test recognition owner stopped before capture opened.")
        })?;
        Ok(Box::new(DropAwareRecognitionCapture {
            dropped: Arc::clone(&opened_capture_dropped),
        }) as Box<dyn RecognitionCapture>)
    };

    let error = match coordinate_running_recognition_with_capture(
        app.handle(),
        &AudioConfig::default(),
        None,
        &generation,
        &mut recognition,
        &status_recorder,
        &open_capture,
    ) {
        Err(error) => error,
        Ok(()) => {
            return Err(AppError::state(
                "Terminal recognition error did not cross the coordinator boundary.",
            ));
        }
    };
    assert_eq!(error.code(), "stt.provider_authentication_failed");
    assert!(capture_dropped.load(Ordering::SeqCst));
    assert!(caption_aggregate.snapshot()?.open_source_units.is_empty());
    let capture_was_dropped = aggregate_closed_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| {
            AppError::state("Terminal open source unit did not close in the aggregate.")
        })?;
    assert!(capture_was_dropped);
    recognition.stop()?;
    generation.request_stop(None)?;
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Running
    );
    Ok(())
}
