//! Recognition ownership and ordered microphone/signal coordination.

use super::RuntimeGeneration;
use super::output::RecognitionEventSubmitOutcome;
use crate::audio::{
    AudioLevelMeter, AudioLevelReading, open_input_capture, speech_gate_level_meter,
};
use crate::chatbox::ChatboxPublication;
use crate::config::AudioConfig;
use crate::error::{AppError, AppResult};
use crate::events::{
    AudioLevelEvent, DiagnosticCategory, DiagnosticUpdate, emit_audio_level, emit_diagnostic,
    emit_runtime_control_changed, record_and_emit_runtime_status,
};
use crate::recognition::{
    OwnedRecognitionAudioFrame, RecognitionModule, RecognitionSignal, RecognitionSubmitError,
    RunningRecognition,
};
use crate::runtime_control::{
    RuntimeStatus, RuntimeStatusRecorder, RuntimeTranslationStatusRecorder,
};
use crate::wall_clock::unix_timestamp_ms;
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::time::Duration;
use tauri::{AppHandle, Runtime};

const RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);

pub(super) struct RuntimeExecution<R: Runtime> {
    app: AppHandle<R>,
    audio_config: AudioConfig,
    recognition_module: RecognitionModule,
    chatbox_publication: Option<ChatboxPublication>,
    generation: RuntimeGeneration,
    status_recorder: RuntimeStatusRecorder,
}

impl<R: Runtime> RuntimeExecution<R> {
    pub(super) fn new(
        app: AppHandle<R>,
        audio_config: AudioConfig,
        recognition_module: RecognitionModule,
        chatbox_publication: Option<ChatboxPublication>,
        generation: RuntimeGeneration,
        status_recorder: RuntimeStatusRecorder,
    ) -> Self {
        Self {
            app,
            audio_config,
            recognition_module,
            chatbox_publication,
            generation,
            status_recorder,
        }
    }

    pub(super) fn app(&self) -> &AppHandle<R> {
        &self.app
    }

    pub(super) fn chatbox_publication(&self) -> Option<&ChatboxPublication> {
        self.chatbox_publication.as_ref()
    }

    pub(super) fn generation(&self) -> &RuntimeGeneration {
        &self.generation
    }

    pub(super) fn status_recorder(&self) -> &RuntimeStatusRecorder {
        &self.status_recorder
    }

    pub(super) fn run(self) -> AppResult<()> {
        let Self {
            app,
            audio_config,
            recognition_module,
            chatbox_publication,
            generation,
            status_recorder,
        } = self;

        let started = generation.commit_if_active(|| {
            record_and_emit_runtime_status(
                &app,
                &status_recorder,
                RuntimeStatus::Starting,
                Some("Starting caption runtime".to_string()),
            );
        })?;
        if !started {
            return Ok(());
        }

        if !generation.accepts_new_work() {
            return Ok(());
        }

        let mut recognition =
            recognition_module.start(crate::recognition::RecognitionGenerationScope {
                generation: generation.generation_id(),
                stream_id: generation.stream_id().to_string(),
            })?;
        let runtime_result = coordinate_running_recognition(
            &app,
            &audio_config,
            chatbox_publication.as_ref(),
            &generation,
            &mut recognition,
            &status_recorder,
        );
        let stop_result = recognition.stop();

        match (runtime_result, stop_result) {
            (Err(runtime_error), Err(stop_error)) => {
                tracing::warn!(
                    code = stop_error.code(),
                    error_message = %stop_error,
                    "Recognition owner also failed while closing after a runtime error"
                );
                Err(runtime_error)
            }
            (Err(runtime_error), Ok(())) => Err(runtime_error),
            (Ok(()), Err(_)) if generation.is_hard_stop_requested() => Ok(()),
            (Ok(()), Err(stop_error)) => Err(stop_error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

struct ActiveRecognitionCapture {
    capture: Box<dyn RecognitionCapture>,
    audio_level: AudioLevelMeter,
}

trait RecognitionCapture {
    fn sample_rate(&self) -> u32;
    fn receive(&self, timeout: Duration) -> AppResult<Option<Vec<f32>>>;
}

struct MicrophoneRecognitionCapture(crate::audio::AudioCapture);

impl RecognitionCapture for MicrophoneRecognitionCapture {
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }

    fn receive(&self, timeout: Duration) -> AppResult<Option<Vec<f32>>> {
        self.0.receive(timeout)
    }
}

fn open_recognition_capture(config: &AudioConfig) -> AppResult<Box<dyn RecognitionCapture>> {
    open_input_capture(config)
        .map(MicrophoneRecognitionCapture)
        .map(|capture| Box::new(capture) as Box<dyn RecognitionCapture>)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecognitionCoordinatorFlow {
    Continue,
    Stopped,
}

struct RecognitionCoordinator<'context, R: Runtime> {
    app: &'context AppHandle<R>,
    audio_config: &'context AudioConfig,
    chatbox_publication: Option<&'context ChatboxPublication>,
    generation: &'context RuntimeGeneration,
    status_recorder: &'context RuntimeStatusRecorder,
    open_capture: &'context dyn Fn(&AudioConfig) -> AppResult<Box<dyn RecognitionCapture>>,
}

fn coordinate_running_recognition<R: Runtime>(
    app: &AppHandle<R>,
    audio_config: &AudioConfig,
    chatbox_publication: Option<&ChatboxPublication>,
    generation: &RuntimeGeneration,
    recognition: &mut RunningRecognition,
    status_recorder: &RuntimeStatusRecorder,
) -> AppResult<()> {
    coordinate_running_recognition_with_capture(
        app,
        audio_config,
        chatbox_publication,
        generation,
        recognition,
        status_recorder,
        &open_recognition_capture,
    )
}

fn coordinate_running_recognition_with_capture<R: Runtime>(
    app: &AppHandle<R>,
    audio_config: &AudioConfig,
    chatbox_publication: Option<&ChatboxPublication>,
    generation: &RuntimeGeneration,
    recognition: &mut RunningRecognition,
    status_recorder: &RuntimeStatusRecorder,
    open_capture: &dyn Fn(&AudioConfig) -> AppResult<Box<dyn RecognitionCapture>>,
) -> AppResult<()> {
    let mut active_capture = None;
    let mut audio_level_revision = 0_u64;
    let mut audio_sequence = 0_u64;
    let coordinator = RecognitionCoordinator {
        app,
        audio_config,
        chatbox_publication,
        generation,
        status_recorder,
        open_capture,
    };
    let translation_status_recorder =
        status_recorder.translation_recorder(generation.generation_id());

    loop {
        if generation.is_work_cancelled() || generation.is_hard_stop_requested() {
            return Ok(());
        }
        drain_translation_outcomes(
            app,
            chatbox_publication,
            generation,
            &translation_status_recorder,
        )?;

        loop {
            let signal = match recognition.signals.try_recv() {
                Ok(signal) => signal,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return finish_unexpected_recognition_owner(
                        coordinator.app,
                        coordinator.chatbox_publication,
                        coordinator.generation,
                        recognition,
                        &mut active_capture,
                    );
                }
            };
            if coordinator.handle_signal(recognition, &mut active_capture, signal)?
                == RecognitionCoordinatorFlow::Stopped
            {
                return Ok(());
            }
            drain_translation_outcomes(
                app,
                chatbox_publication,
                generation,
                &translation_status_recorder,
            )?;
        }

        let Some(active) = active_capture.as_mut() else {
            let signal = match recognition.signals.recv_timeout(RECEIVE_TIMEOUT) {
                Ok(signal) => signal,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return finish_unexpected_recognition_owner(
                        coordinator.app,
                        coordinator.chatbox_publication,
                        coordinator.generation,
                        recognition,
                        &mut active_capture,
                    );
                }
            };
            if coordinator.handle_signal(recognition, &mut active_capture, signal)?
                == RecognitionCoordinatorFlow::Stopped
            {
                return Ok(());
            }
            drain_translation_outcomes(
                app,
                chatbox_publication,
                generation,
                &translation_status_recorder,
            )?;
            continue;
        };

        let Some(samples) = active.capture.receive(RECEIVE_TIMEOUT)? else {
            continue;
        };
        if !recognition.is_accepting_audio() {
            drop(active_capture.take());
            continue;
        }
        let level_readings: Vec<AudioLevelReading> = active.audio_level.push_samples(&samples);
        for reading in level_readings {
            let next_revision = audio_level_revision.saturating_add(1);
            let accepted = generation.commit_if_active(|| {
                emit_audio_level(
                    app,
                    AudioLevelEvent {
                        generation: generation.generation_id(),
                        revision: next_revision,
                        rms_dbfs: reading.rms_dbfs,
                        peak_dbfs: reading.peak_dbfs,
                        clipping: reading.clipping,
                        gate_open: reading.vad_gate_open,
                        timestamp_ms: unix_timestamp_ms(),
                    },
                );
            })?;
            if !accepted {
                return Ok(());
            }
            audio_level_revision = next_revision;
        }

        audio_sequence = audio_sequence
            .checked_add(1)
            .ok_or_else(|| AppError::state("Recognition audio sequence was exhausted."))?;
        match recognition.try_submit(OwnedRecognitionAudioFrame {
            sequence: audio_sequence,
            captured_at_ms: unix_timestamp_ms(),
            sample_rate_hz: active.capture.sample_rate(),
            samples: samples.into_boxed_slice(),
        }) {
            Ok(()) => {}
            Err(RecognitionSubmitError::Backpressure) => {
                return Err(AppError::recognition_backpressure(
                    "The active recognizer could not keep up with microphone audio; its bounded audio budget filled, so captioning stopped instead of silently dropping audio.",
                ));
            }
            Err(RecognitionSubmitError::InvalidAudio) => {
                return Err(AppError::audio(
                    "The microphone produced an invalid recognition audio frame.",
                ));
            }
            Err(RecognitionSubmitError::NotReady) => {
                drop(active_capture.take());
            }
            Err(RecognitionSubmitError::Stopped)
                if generation.is_work_cancelled() || generation.is_hard_stop_requested() =>
            {
                return Ok(());
            }
            Err(RecognitionSubmitError::Stopped) => {
                // The owner closes audio admission only after its signal sender
                // has released all already-emitted events. Retire capture, then
                // loop once more so those ordered signals are consumed before
                // the disconnected lane is joined and reported.
                drop(active_capture.take());
            }
        }
    }
}

impl<R: Runtime> RecognitionCoordinator<'_, R> {
    fn handle_signal(
        &self,
        recognition: &RunningRecognition,
        active_capture: &mut Option<ActiveRecognitionCapture>,
        signal: RecognitionSignal,
    ) -> AppResult<RecognitionCoordinatorFlow> {
        let app = self.app;
        let audio_config = self.audio_config;
        let chatbox_publication = self.chatbox_publication;
        let generation = self.generation;
        let status_recorder = self.status_recorder;
        let open_capture = self.open_capture;

        match signal {
            RecognitionSignal::Ready {
                generation: signal_generation,
                stream_id,
                recovered,
            } => {
                if signal_generation != generation.generation_id()
                    || stream_id != generation.stream_id()
                {
                    return Err(AppError::state(
                        "Recognition owner emitted Ready for the wrong runtime generation.",
                    ));
                }
                if active_capture.is_some() {
                    return Err(AppError::state(
                        "Recognition owner emitted Ready while microphone capture was already active.",
                    ));
                }

                let capture = open_capture(audio_config)?;
                let sample_rate = capture.sample_rate();
                let audio_level = speech_gate_level_meter(sample_rate)?;
                if !recognition.is_accepting_audio() {
                    drop(capture);
                    return Ok(RecognitionCoordinatorFlow::Continue);
                }

                let running = generation.commit_if_active(|| {
                    record_and_emit_runtime_status(
                        app,
                        status_recorder,
                        RuntimeStatus::Running,
                        Some("Listening for microphone speech".to_string()),
                    );
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::info(
                            DiagnosticCategory::Audio,
                            "audio.capture_started",
                            "Microphone capture started",
                            format!("Capturing mono audio at {sample_rate} Hz."),
                        ),
                    );
                    if recovered {
                        emit_diagnostic(
                            app,
                            DiagnosticUpdate::info(
                                DiagnosticCategory::Recognition,
                                "stt.reconnected",
                                "Recognition connection restored",
                                "Microphone capture resumed with a fresh recognition attempt. No prior audio was replayed.",
                            ),
                        );
                    }
                })?;
                if !running {
                    drop(capture);
                    return Ok(RecognitionCoordinatorFlow::Stopped);
                }
                *active_capture = Some(ActiveRecognitionCapture {
                    capture,
                    audio_level,
                });
                Ok(RecognitionCoordinatorFlow::Continue)
            }
            RecognitionSignal::Reconnecting {
                epoch,
                retry_number,
                delay_ms,
            } => {
                drop(active_capture.take());
                generation.abort_open_source_units_for_reconnect(app, chatbox_publication)?;
                let reconnecting = generation.commit_if_active(|| {
                    record_and_emit_runtime_status(
                        app,
                        status_recorder,
                        RuntimeStatus::Reconnecting,
                        Some(format!(
                            "Recognition connection interrupted; retry {retry_number} in {delay_ms} ms"
                        )),
                    );
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::warning(
                            DiagnosticCategory::Recognition,
                            "stt.reconnecting",
                            "Recognition connection interrupted",
                            format!(
                                "Microphone capture is paused; retry {retry_number} begins in {delay_ms} ms. Unconfirmed speech was discarded."
                            ),
                        ),
                    );
                })?;
                recognition.acknowledge_capture_paused(epoch)?;
                Ok(if reconnecting {
                    RecognitionCoordinatorFlow::Continue
                } else {
                    RecognitionCoordinatorFlow::Stopped
                })
            }
            RecognitionSignal::Event(event) => {
                let outcome =
                    generation.submit_recognition_event(app, chatbox_publication, event)?;
                if let RecognitionEventSubmitOutcome::AcceptedWithTranslationFailure(reason) =
                    outcome
                {
                    record_translation_degradation(
                        app,
                        &status_recorder.translation_recorder(generation.generation_id()),
                        reason,
                    )?;
                }
                Ok(if outcome == RecognitionEventSubmitOutcome::Stopped {
                    RecognitionCoordinatorFlow::Stopped
                } else {
                    RecognitionCoordinatorFlow::Continue
                })
            }
        }
    }
}

fn drain_translation_outcomes<R: Runtime>(
    app: &AppHandle<R>,
    chatbox_publication: Option<&ChatboxPublication>,
    generation: &RuntimeGeneration,
    recorder: &RuntimeTranslationStatusRecorder,
) -> AppResult<()> {
    if let Some(reason) = generation
        .drain_translation_outcomes(app, chatbox_publication)?
        .degradation
    {
        record_translation_degradation(app, recorder, reason)?;
    }
    Ok(())
}

fn record_translation_degradation<R: Runtime>(
    app: &AppHandle<R>,
    recorder: &RuntimeTranslationStatusRecorder,
    reason: crate::caption::TranslationFailureReason,
) -> AppResult<()> {
    if let Some(snapshot) = recorder.record_degraded(reason)? {
        emit_runtime_control_changed(app, snapshot);
    }
    Ok(())
}

fn finish_unexpected_recognition_owner<R: Runtime>(
    app: &AppHandle<R>,
    chatbox_publication: Option<&ChatboxPublication>,
    generation: &RuntimeGeneration,
    recognition: &mut RunningRecognition,
    active_capture: &mut Option<ActiveRecognitionCapture>,
) -> AppResult<()> {
    drop(active_capture.take());
    let owner_error = recognition.stop().err().unwrap_or_else(|| {
        AppError::runtime(
            "Recognition Module owner stopped unexpectedly while the runtime was active.",
        )
    });
    if let Err(cleanup_error) =
        generation.abort_open_source_units_for_terminal_failure(app, chatbox_publication)
    {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(
                &cleanup_error,
                "Active speech could not resolve after recognition stopped",
            ),
        );
    }
    Err(owner_error)
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
