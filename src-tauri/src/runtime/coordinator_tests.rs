use super::*;
use crate::caption::{CaptionAggregateSnapshot, CaptionAggregateStore, TranslationFailureReason};
use crate::caption_pipeline::plan_caption_pipeline;
use crate::config::{
    AppConfig, ContentSelection, PublicationConfig, TranslationConfig, TranslationEndpoint,
    TranslationPath, TranslationTarget,
};
use crate::recognition::{
    RecognitionDriver, RecognitionDriverInput, RecognitionDriverIo, RecognitionEvent,
    RecognitionGenerationScope, RecognitionModule,
};
use crate::runtime_control::{
    ChatboxPublicationSnapshot, RuntimeControlStore, RuntimeGenerationPhase,
    RuntimeGenerationSelection, RuntimeGenerationSnapshot, RuntimeGenerationTranslationState,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use tauri::Listener;

const TEST_WATCHDOG: Duration = Duration::from_secs(5);

enum LifecycleDriverPlan {
    ReadyUntilStopped,
    FailAfterRelease { release: mpsc::Receiver<()> },
    FailAfterFirstAudio { capture: CaptureProbe },
    ReconnectAfterFirstAudio { capture: CaptureProbe },
}

struct LifecycleRecognitionDriver {
    plan: LifecycleDriverPlan,
}

struct DropTrackedRecognitionDriver {
    dropped: Arc<AtomicBool>,
}

#[test]
fn translation_degradation_is_recorded_without_marking_the_runtime_failed() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let config = AppConfig {
        translation: Some(TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::SimplifiedChinese,
            endpoint: TranslationEndpoint::Official,
        }),
        publication: PublicationConfig {
            content: ContentSelection::Bilingual,
            ..PublicationConfig::default()
        },
        ..AppConfig::default()
    };
    control.install_starting_generation(RuntimeGenerationSnapshot {
        id: 1,
        phase: RuntimeGenerationPhase::Running,
        started_from_config_revision: 0,
        selection: RuntimeGenerationSelection::from(&config),
        caption_pipeline_plan: plan_caption_pipeline(&config),
        credentials: Vec::new(),
        chatbox_publication: ChatboxPublicationSnapshot::Disabled {
            host: config.osc.host.clone(),
            port: config.osc.port,
        },
        translation_state: RuntimeGenerationTranslationState::Active,
        uploads_microphone_audio: false,
        uploads_source_text: true,
    })?;
    let status_recorder = control.status_recorder();

    record_translation_degradation(
        app.handle(),
        &status_recorder.translation_recorder(1),
        TranslationFailureReason::ProviderUnavailable,
    )?;

    let snapshot = control.snapshot()?;
    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Starting);
    assert!(matches!(
        snapshot
            .generation
            .as_ref()
            .map(|generation| &generation.translation_state),
        Some(RuntimeGenerationTranslationState::Degraded {
            reason_code: TranslationFailureReason::ProviderUnavailable,
        })
    ));
    Ok(())
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

impl RecognitionDriver for LifecycleRecognitionDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        io.ready(false)?;
        match self.plan {
            LifecycleDriverPlan::ReadyUntilStopped => io.wait_until_stopped(),
            LifecycleDriverPlan::FailAfterRelease { release } => {
                release.recv_timeout(TEST_WATCHDOG).map_err(|error| {
                    AppError::state(format!(
                        "Capture opening did not release the Recognition owner: {error}"
                    ))
                })?;
                Err(terminal_authentication_error())
            }
            LifecycleDriverPlan::FailAfterFirstAudio { capture } => {
                if !receive_first_audio(&io)? {
                    return Ok(());
                }
                capture.mark_audio_admitted();
                io.emit_event(RecognitionEvent::UnitStarted {
                    generation: io.scope().generation,
                    stream_id: io.scope().stream_id.clone(),
                    unit_id: "terminal-active-unit".to_string(),
                    started_at_ms: 321,
                })?;
                Err(terminal_authentication_error())
            }
            LifecycleDriverPlan::ReconnectAfterFirstAudio { capture } => {
                if !receive_first_audio(&io)? {
                    return Ok(());
                }
                capture.mark_audio_admitted();
                io.reconnecting(7, 1, Duration::from_millis(10))?;
                io.wait_until_stopped()
            }
        }
    }
}

fn receive_first_audio(io: &RecognitionDriverIo) -> AppResult<bool> {
    match io.receive(TEST_WATCHDOG)? {
        RecognitionDriverInput::Audio(_) => Ok(true),
        RecognitionDriverInput::Stopped => Ok(false),
        RecognitionDriverInput::Idle => Err(AppError::state(
            "Active capture did not admit audio before the test watchdog expired.",
        )),
    }
}

fn terminal_authentication_error() -> AppError {
    AppError::recognition_provider(
        crate::error::ProviderFailureClass::Authentication,
        "The recognition provider rejected the configured credential.",
    )
}

#[derive(Clone, Default)]
struct CaptureProbe {
    returned: Arc<AtomicBool>,
    first_receive_entered: Arc<AtomicBool>,
    audio_admitted: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl CaptureProbe {
    fn mark_returned(&self) {
        self.returned.store(true, Ordering::SeqCst);
    }

    fn begin_receive(&self, control: &RuntimeControlStore) -> AppResult<bool> {
        if self.first_receive_entered.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        if control.snapshot()?.runtime_status.status != RuntimeStatus::Running {
            return Err(AppError::state(
                "Microphone capture received audio before Runtime committed Running.",
            ));
        }
        Ok(true)
    }

    fn mark_audio_admitted(&self) {
        self.audio_admitted.store(true, Ordering::SeqCst);
    }

    fn mark_dropped(&self) {
        self.dropped.store(true, Ordering::SeqCst);
    }

    fn was_returned(&self) -> bool {
        self.returned.load(Ordering::SeqCst)
    }

    fn receive_was_entered(&self) -> bool {
        self.first_receive_entered.load(Ordering::SeqCst)
    }

    fn audio_was_admitted(&self) -> bool {
        self.audio_admitted.load(Ordering::SeqCst)
    }

    fn was_dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }
}

struct OneFrameRecognitionCapture {
    control: RuntimeControlStore,
    probe: CaptureProbe,
}

struct HardStopRecognitionCapture {
    control: RuntimeControlStore,
    generation: RuntimeGeneration,
    probe: CaptureProbe,
}

impl RecognitionCapture for OneFrameRecognitionCapture {
    fn sample_rate(&self) -> u32 {
        16_000
    }

    fn receive(&self, _timeout: Duration) -> AppResult<Option<Vec<f32>>> {
        if !self.probe.begin_receive(&self.control)? {
            return Ok(None);
        }
        Ok(Some(vec![0.0; 160]))
    }
}

impl Drop for OneFrameRecognitionCapture {
    fn drop(&mut self) {
        self.probe.mark_dropped();
    }
}

impl Drop for HardStopRecognitionCapture {
    fn drop(&mut self) {
        self.probe.mark_dropped();
    }
}

impl RecognitionCapture for HardStopRecognitionCapture {
    fn sample_rate(&self) -> u32 {
        16_000
    }

    fn receive(&self, _timeout: Duration) -> AppResult<Option<Vec<f32>>> {
        if self.probe.begin_receive(&self.control)? {
            self.generation.request_stop(None)?;
        }
        Ok(None)
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
    let capture = CaptureProbe::default();
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        LifecycleRecognitionDriver {
            plan: LifecycleDriverPlan::ReconnectAfterFirstAudio {
                capture: capture.clone(),
            },
        },
    )?;
    let mut recognition = module.start(RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let opened_capture = capture.clone();
    let capture_control = control.clone();
    let open_capture = move |_config: &AudioConfig| {
        opened_capture.mark_returned();
        Ok(Box::new(OneFrameRecognitionCapture {
            control: capture_control.clone(),
            probe: opened_capture.clone(),
        }) as Box<dyn RecognitionCapture>)
    };
    let capture_at_acknowledgement = capture.clone();
    let stop_generation = generation.clone();
    let before_capture_pause_acknowledgement = move || {
        if !capture_at_acknowledgement.was_dropped() {
            return Err(AppError::state(
                "Reconnect acknowledged capture pause before microphone capture was dropped.",
            ));
        }
        stop_generation.request_stop(None)
    };

    coordinate_running_recognition_with_capture_adapter(
        app.handle(),
        &AudioConfig::default(),
        None,
        &generation,
        &mut recognition,
        &status_recorder,
        RecognitionCaptureAdapter::with_pause_acknowledgement(
            &open_capture,
            &before_capture_pause_acknowledgement,
        ),
    )?;
    recognition.stop()?;
    assert!(capture.was_returned());
    assert!(capture.receive_was_entered());
    assert!(capture.audio_was_admitted());
    assert!(capture.was_dropped());
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Reconnecting
    );
    Ok(())
}

#[test]
fn hard_stop_drops_active_capture_before_coordinator_returns() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let generation =
        RuntimeGeneration::activate(app.handle(), 1, CaptionAggregateStore::default())?;
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        LifecycleRecognitionDriver {
            plan: LifecycleDriverPlan::ReadyUntilStopped,
        },
    )?;
    let mut recognition = module.start(RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let capture = CaptureProbe::default();
    let opened_capture = capture.clone();
    let capture_control = control.clone();
    let stop_generation = generation.clone();
    let open_capture = move |_config: &AudioConfig| {
        opened_capture.mark_returned();
        Ok(Box::new(HardStopRecognitionCapture {
            control: capture_control.clone(),
            generation: stop_generation.clone(),
            probe: opened_capture.clone(),
        }) as Box<dyn RecognitionCapture>)
    };

    coordinate_running_recognition_with_capture(
        app.handle(),
        &AudioConfig::default(),
        None,
        &generation,
        &mut recognition,
        &status_recorder,
        &open_capture,
    )?;

    assert!(generation.is_hard_stop_requested());
    assert!(capture.was_returned());
    assert!(capture.receive_was_entered());
    assert!(capture.was_dropped());
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Running
    );
    recognition.stop()?;
    Ok(())
}

#[test]
fn owner_termination_before_capture_activation_does_not_commit_running() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let generation =
        RuntimeGeneration::activate(app.handle(), 1, CaptionAggregateStore::default())?;
    let (capture_open_entered, capture_open_entry) = mpsc::sync_channel(1);
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        LifecycleRecognitionDriver {
            plan: LifecycleDriverPlan::FailAfterRelease {
                release: capture_open_entry,
            },
        },
    )?;
    let mut recognition = module.start(RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let owner_termination = recognition.owner_termination_observer();
    let capture = CaptureProbe::default();
    let opened_capture = capture.clone();
    let capture_control = control.clone();
    let open_capture = move |_config: &AudioConfig| {
        capture_open_entered.send(()).map_err(|_| {
            AppError::state("Recognition owner stopped before capture opening was observed.")
        })?;
        owner_termination.wait_for_termination(TEST_WATCHDOG)?;
        opened_capture.mark_returned();
        Ok(Box::new(OneFrameRecognitionCapture {
            control: capture_control.clone(),
            probe: opened_capture.clone(),
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
    assert_eq!(
        error.provider_failure_class(),
        Some(crate::error::ProviderFailureClass::Authentication)
    );
    assert!(capture.was_returned());
    assert!(capture.was_dropped());
    assert!(!capture.receive_was_entered());
    assert!(!capture.audio_was_admitted());
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Idle
    );
    recognition.stop()?;
    generation.request_stop(None)?;
    Ok(())
}

#[test]
fn owner_termination_after_capture_activation_drops_capture_and_closes_source_unit() -> AppResult<()>
{
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
    let capture = CaptureProbe::default();
    let source_unit_opened = Arc::new(AtomicBool::new(false));
    let observed_capture = capture.clone();
    let observed_source_unit_opened = Arc::clone(&source_unit_opened);
    let (aggregate_closed_sender, aggregate_closed_receiver) = mpsc::channel();
    app.listen("caption-aggregate-changed", move |event| {
        let Ok(snapshot) = serde_json::from_str::<CaptionAggregateSnapshot>(event.payload()) else {
            return;
        };
        if !snapshot.open_source_units.is_empty() {
            observed_source_unit_opened.store(true, Ordering::SeqCst);
        } else if observed_source_unit_opened.load(Ordering::SeqCst) {
            let _ = aggregate_closed_sender.send(observed_capture.was_dropped());
        }
    });
    let module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        LifecycleRecognitionDriver {
            plan: LifecycleDriverPlan::FailAfterFirstAudio {
                capture: capture.clone(),
            },
        },
    )?;
    let mut recognition = module.start(RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let owner_termination = recognition.owner_termination_observer();
    let opened_capture = capture.clone();
    let capture_control = control.clone();
    let open_capture = move |_config: &AudioConfig| {
        opened_capture.mark_returned();
        Ok(Box::new(OneFrameRecognitionCapture {
            control: capture_control.clone(),
            probe: opened_capture.clone(),
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
    owner_termination.wait_for_termination(TEST_WATCHDOG)?;
    assert_eq!(error.code(), "stt.provider_authentication_failed");
    assert_eq!(
        error.provider_failure_class(),
        Some(crate::error::ProviderFailureClass::Authentication)
    );
    assert!(capture.was_returned());
    assert!(capture.receive_was_entered());
    assert!(capture.audio_was_admitted());
    assert!(capture.was_dropped());
    assert!(source_unit_opened.load(Ordering::SeqCst));
    assert!(caption_aggregate.snapshot()?.open_source_units.is_empty());
    let capture_was_dropped = aggregate_closed_receiver
        .recv_timeout(TEST_WATCHDOG)
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
