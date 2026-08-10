//! Runtime lifecycle for outgoing captions.
//!
//! The runtime owns one microphone, one active Recognition Module, and one
//! publication policy per generation. Runtime forwards continuous microphone
//! audio and consumes normalized recognition signals; provider adapters own
//! speech units, connection attempts, and protocol I/O.
//!
//! Every utterance announced with `utterance-started` resolves with either a
//! completed caption in the caption-session aggregate or an `utterance-ended`
//! event, so the UI never waits on recognition that cannot arrive. Listening
//! indicators remain distinct lifecycle events rather than placeholder text.
//!
//! Stop is a hard cutoff: the microphone is released within one receive timeout,
//! buffered and queued speech is discarded instead of drained, and no App or
//! Chatbox caption text is committed after the stop request. A state-clearing
//! typing-off packet is sent before waiting for an STT request that is already
//! in flight, so runtime commands must run off the main thread
//! (`#[tauri::command(async)]`) to keep the window responsive during that wait.

mod output;

pub(crate) use output::{ChatboxPublisherBoundary, RuntimeGeneration};

use output::{
    RecognitionEventSubmitOutcome, RuntimePublisherInit, finish_runtime_output,
    initialize_runtime_publisher, publisher_boundary, publisher_failure_message,
};

use crate::audio::{
    AudioLevelMeter, AudioLevelReading, open_input_capture, speech_gate_level_meter,
};
use crate::capability_planner::{ResolvedPublicationPolicy, RuntimePlanSnapshot, plan_runtime};
use crate::caption_session::CaptionSessionStore;
use crate::chatbox::{ChatboxPacer, PublisherCloseReason, RuntimeChatboxPublisher};
#[cfg(test)]
use crate::chatbox::{
    CompletedChatboxPublisher, CompletedPublisherEvent, LiveChatboxPublisher,
    LivePublisherReporter, PublisherReporter, PublisherSubmitOutcome,
    completed_publisher_diagnostic, live_publisher_diagnostic,
};
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::events::{
    AudioLevelEvent, DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, emit_audio_level,
    emit_diagnostic, now_ms, record_and_emit_runtime_status,
};
use crate::host_resolver::HostResolver;
use crate::recognition::{
    OwnedRecognitionAudioFrame, RecognitionSignal, RecognitionSubmitError, RunningRecognition,
    openai_recognition_module,
};
#[cfg(test)]
use crate::recognition::{RecognitionEndReason, RecognitionEvent};
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeCredentialSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase,
    RuntimeSessionSnapshot,
};
use secrecy::SecretString;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tauri::{AppHandle, Runtime};

const RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) struct RuntimeManager {
    handle: Mutex<Option<RuntimeHandle>>,
    stop_epoch: AtomicU64,
    audio_probe_active: AtomicBool,
}

pub(crate) struct AudioProbeLease<'a> {
    active: &'a AtomicBool,
}

impl Drop for AudioProbeLease<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct RuntimeStartRequest {
    pub(crate) config: AppConfig,
    pub(crate) chatbox_pacer: ChatboxPacer,
    pub(crate) caption_session: CaptionSessionStore,
    pub(crate) host_resolver: HostResolver,
    pub(crate) generation_id: u64,
    pub(crate) config_revision: u64,
    pub(crate) openai_api_key: SecretString,
    pub(crate) credential: RuntimeCredentialSnapshot,
    pub(crate) expected_stop_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStartOutcome {
    Started,
    SupersededByStop,
}

struct RuntimeHandle {
    generation: RuntimeGeneration,
    publisher: Option<RuntimeChatboxPublisher>,
    join_handle: JoinHandle<()>,
}

fn resolve_runtime_publication_policy(
    runtime_plan: &RuntimePlanSnapshot,
) -> AppResult<ResolvedPublicationPolicy> {
    runtime_plan.publication.resolved_policy().ok_or_else(|| {
        AppError::config(format!(
            "The selected recognition path and publication mode are incompatible ({}).",
            runtime_plan
                .publication
                .incompatibility_code()
                .unwrap_or("publication.incompatible")
        ))
    })
}

impl Default for RuntimeManager {
    fn default() -> Self {
        Self {
            handle: Mutex::new(None),
            stop_epoch: AtomicU64::new(0),
            audio_probe_active: AtomicBool::new(false),
        }
    }
}

impl RuntimeManager {
    pub(crate) fn stop_epoch(&self) -> u64 {
        self.stop_epoch.load(Ordering::SeqCst)
    }

    pub(crate) fn stop_epoch_unchanged(&self, expected_stop_epoch: u64) -> bool {
        self.stop_epoch() == expected_stop_epoch
    }

    pub(crate) fn prepare_for_start<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(app, &mut guard)?;

        if self.audio_probe_active.load(Ordering::SeqCst) {
            return Err(AppError::runtime(
                "A microphone test is already using the selected audio input.",
            ));
        }
        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        Ok(())
    }

    pub(crate) fn begin_audio_probe<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> AppResult<AudioProbeLease<'_>> {
        self.audio_probe_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| AppError::runtime("A microphone test is already running."))?;
        let lease = AudioProbeLease {
            active: &self.audio_probe_active,
        };
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(app, &mut guard)?;
        if guard.is_some() {
            return Err(AppError::runtime(
                "Stop the caption runtime before testing the microphone.",
            ));
        }
        Ok(lease)
    }

    pub(crate) fn start<R: Runtime, F>(
        &self,
        app: AppHandle<R>,
        request: RuntimeStartRequest,
        install_session: F,
    ) -> AppResult<RuntimeStartOutcome>
    where
        F: FnOnce(RuntimeSessionSnapshot) -> AppResult<()>,
    {
        let RuntimeStartRequest {
            config,
            chatbox_pacer,
            caption_session,
            host_resolver,
            generation_id,
            config_revision,
            openai_api_key,
            credential,
            expected_stop_epoch,
        } = request;
        config.validate()?;
        let runtime_plan = plan_runtime(&config);
        let publication_policy = resolve_runtime_publication_policy(&runtime_plan)?;

        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(&app, &mut guard)?;

        // Stop increments its epoch before waiting for this handle lock. The
        // comparison and handle installation are therefore one linearized
        // decision: an earlier Start cannot come back to life after Stop has
        // already returned while Start was resolving slow desired-state I/O.
        if !self.stop_epoch_unchanged(expected_stop_epoch) {
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }
        if self.audio_probe_active.load(Ordering::SeqCst) {
            return Err(AppError::runtime(
                "A microphone test is already using the selected audio input.",
            ));
        }

        let generation = RuntimeGeneration::activate(&app, generation_id, caption_session)?;
        let start_cancelled = || !self.stop_epoch_unchanged(expected_stop_epoch);
        let publisher_init = initialize_runtime_publisher(
            &app,
            &config.osc,
            publication_policy,
            chatbox_pacer,
            generation.clone(),
            &host_resolver,
            &start_cancelled,
        );
        if start_cancelled() {
            match &publisher_init {
                RuntimePublisherInit::Ready(publisher) => {
                    let _ = generation.request_stop(Some(publisher));
                    let _ = publisher.join();
                }
                RuntimePublisherInit::Disabled | RuntimePublisherInit::Unavailable(_) => {
                    let _ = generation.request_stop(None);
                }
            }
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }
        let requested_host = config.osc.host.clone();
        let requested_port = config.osc.port;
        let (publisher, chatbox) = match publisher_init {
            RuntimePublisherInit::Disabled => (
                None,
                RuntimeChatboxSnapshot::Disabled {
                    host: requested_host,
                    port: requested_port,
                },
            ),
            RuntimePublisherInit::Ready(publisher) => (
                Some(publisher),
                RuntimeChatboxSnapshot::Ready {
                    host: requested_host,
                    port: requested_port,
                },
            ),
            RuntimePublisherInit::Unavailable(error) => {
                emit_diagnostic(
                    &app,
                    DiagnosticUpdate::from_error(&error, "Chatbox OSC output could not start"),
                );
                (
                    None,
                    RuntimeChatboxSnapshot::Unavailable {
                        host: requested_host,
                        port: requested_port,
                        reason_code: error.code().to_string(),
                    },
                )
            }
        };

        let session = RuntimeSessionSnapshot {
            generation: generation_id,
            phase: RuntimeSessionPhase::Starting,
            started_from_config_revision: config_revision,
            selected: RuntimeSelectedConfig::from(&config),
            runtime_plan,
            credential: Some(credential),
            chatbox,
            uploads_microphone_audio: true,
        };
        if let Err(error) = install_session(session) {
            let _ = generation.request_stop(publisher_boundary(publisher.as_ref()));
            if let Some(publisher) = &publisher {
                let _ = publisher.join();
            }
            return Err(error);
        }

        let thread_generation = generation.clone();
        let thread_publisher = publisher.clone();
        let join_handle = thread::Builder::new()
            .name("vrc-live-caption-runtime".to_string())
            .spawn(move || {
                run_runtime_thread(
                    app,
                    config,
                    openai_api_key,
                    thread_publisher,
                    thread_generation,
                    host_resolver,
                )
            })
            .map_err(|error| AppError::runtime(format!("Failed to start runtime thread: {error}")));
        let join_handle = match join_handle {
            Ok(join_handle) => join_handle,
            Err(error) => {
                let _ = generation.request_stop(publisher_boundary(publisher.as_ref()));
                if let Some(publisher) = &publisher {
                    let _ = publisher.join();
                }
                return Err(error);
            }
        };

        *guard = Some(RuntimeHandle {
            generation,
            publisher,
            join_handle,
        });

        Ok(RuntimeStartOutcome::Started)
    }

    pub(crate) fn stop<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        // Publish the stop intent before waiting for the handle. A Start that
        // has not committed its handle yet observes the changed epoch and
        // aborts; a Start already inside the handle lock is stopped below.
        self.stop_epoch.fetch_add(1, Ordering::SeqCst);

        // Hold the lock through the join so a concurrent start cannot spawn a
        // new runtime while the old worker is still finishing its last request.
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;

        let Some(handle) = guard.take() else {
            record_and_emit_runtime_status(
                app,
                RuntimeStatus::Stopped,
                Some("Runtime is already stopped".to_string()),
            );
            return Ok(());
        };

        if let Err(error) = handle
            .generation
            .request_stop(publisher_boundary(handle.publisher.as_ref()))
        {
            handle.generation.cancel_work();
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        handle.publisher.as_ref(),
                        "Completed publisher could not close",
                        "Live publisher could not close",
                    ),
                ),
            );
        }
        record_and_emit_runtime_status(
            app,
            RuntimeStatus::Stopping,
            Some("Stopping runtime and discarding pending speech".to_string()),
        );

        let publisher_result = match &handle.publisher {
            Some(publisher) => publisher.join(),
            None => Ok(()),
        };
        let runtime_panicked = handle.join_handle.join().is_err();

        if let Err(error) = publisher_result {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        handle.publisher.as_ref(),
                        "Completed publisher failed while stopping",
                        "Live publisher failed while stopping",
                    ),
                ),
            );
        }

        if runtime_panicked {
            let error = AppError::runtime("Runtime thread panicked while stopping.");
            record_and_emit_runtime_status(app, RuntimeStatus::Error, Some(error.to_string()));
            return Err(error);
        }

        record_and_emit_runtime_status(
            app,
            RuntimeStatus::Stopped,
            Some("Runtime stopped".to_string()),
        );
        emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Runtime,
                "runtime.stopped",
                "Runtime stopped",
                "Microphone capture has been released.",
            ),
        );

        Ok(())
    }
}

fn clear_finished_runtime<R: Runtime>(
    app: &AppHandle<R>,
    handle: &mut Option<RuntimeHandle>,
) -> AppResult<()> {
    let is_finished = handle
        .as_ref()
        .map(|handle| handle.join_handle.is_finished())
        .unwrap_or(false);

    if !is_finished {
        return Ok(());
    }

    let Some(handle) = handle.take() else {
        return Ok(());
    };

    if let Some(publisher) = &handle.publisher {
        if let Err(error) = handle
            .generation
            .close_publisher_at_boundary(Some(publisher), PublisherCloseReason::RuntimeError)
        {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        Some(publisher),
                        "Completed publisher could not close",
                        "Live publisher could not close",
                    ),
                ),
            );
        }
        if let Err(error) = publisher.join() {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        Some(publisher),
                        "Completed publisher failed while closing",
                        "Live publisher failed while closing",
                    ),
                ),
            );
        }
    }

    handle
        .join_handle
        .join()
        .map_err(|_| AppError::runtime("Runtime thread panicked after stopping."))
}

fn run_runtime_thread<R: Runtime>(
    app: AppHandle<R>,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    host_resolver: HostResolver,
) {
    let error_generation = generation.clone();
    let cleanup_publisher = publisher.clone();
    let runtime_app = app.clone();

    supervise_runtime_thread(
        &app,
        &error_generation,
        cleanup_publisher.as_ref(),
        move || {
            run_runtime(
                runtime_app,
                config,
                openai_api_key,
                publisher,
                generation,
                host_resolver,
            )
        },
    );
}

fn supervise_runtime_thread<R: Runtime>(
    app: &AppHandle<R>,
    generation: &RuntimeGeneration,
    publisher: Option<&RuntimeChatboxPublisher>,
    run: impl FnOnce() -> AppResult<()>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    let runtime_result = match outcome {
        Ok(runtime_result) => runtime_result,
        Err(panic) => {
            finish_runtime_output(app, generation, publisher, PublisherCloseReason::Stop);
            tracing::error!("runtime thread panicked; its generation and Publisher were stopped");
            record_and_emit_runtime_status(
                app,
                RuntimeStatus::Error,
                Some("Runtime thread panicked and was stopped".to_string()),
            );
            emit_diagnostic(
                app,
                DiagnosticUpdate::error(
                    DiagnosticCategory::Runtime,
                    "runtime.thread_panicked",
                    "Runtime thread panicked",
                    "The runtime generation was invalidated and pending Chatbox output was discarded.",
                ),
            );
            std::panic::resume_unwind(panic);
        }
    };

    let reason = if generation.is_hard_stop_requested() {
        PublisherCloseReason::Stop
    } else {
        PublisherCloseReason::RuntimeError
    };
    finish_runtime_output(app, generation, publisher, reason);

    if let Err(error) = runtime_result {
        tracing::warn!(
            code = error.code(),
            error_message = %error,
            "runtime stopped with error"
        );

        if generation.is_hard_stop_requested() {
            return;
        }

        record_and_emit_runtime_status(app, RuntimeStatus::Error, Some(error.to_string()));
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Runtime stopped with an error"),
        );
    }
}

fn run_runtime<R: Runtime>(
    app: AppHandle<R>,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    host_resolver: HostResolver,
) -> AppResult<()> {
    let started = generation.commit_if_active(|| {
        record_and_emit_runtime_status(
            &app,
            RuntimeStatus::Starting,
            Some("Starting outgoing caption runtime".to_string()),
        );
    })?;
    if !started {
        return Ok(());
    }

    run_openai_runtime(
        app,
        config,
        openai_api_key,
        publisher,
        generation,
        host_resolver,
    )
}

fn run_openai_runtime<R: Runtime>(
    app: AppHandle<R>,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    host_resolver: HostResolver,
) -> AppResult<()> {
    if !generation.accepts_new_work() {
        return Ok(());
    }

    let module = openai_recognition_module(
        config.stt.model,
        config.stt.languages.clone(),
        openai_api_key,
        host_resolver,
    )?;
    let mut recognition = module.start(crate::recognition::RecognitionGenerationScope {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    })?;
    let runtime_result = coordinate_running_recognition(
        &app,
        &config,
        publisher.as_ref(),
        &generation,
        &mut recognition,
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

fn open_recognition_capture(
    config: &crate::config::AudioConfig,
) -> AppResult<Box<dyn RecognitionCapture>> {
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
    config: &'context AppConfig,
    publisher: Option<&'context RuntimeChatboxPublisher>,
    generation: &'context RuntimeGeneration,
    open_capture:
        &'context dyn Fn(&crate::config::AudioConfig) -> AppResult<Box<dyn RecognitionCapture>>,
}

fn coordinate_running_recognition<R: Runtime>(
    app: &AppHandle<R>,
    config: &AppConfig,
    publisher: Option<&RuntimeChatboxPublisher>,
    generation: &RuntimeGeneration,
    recognition: &mut RunningRecognition,
) -> AppResult<()> {
    coordinate_running_recognition_with_capture(
        app,
        config,
        publisher,
        generation,
        recognition,
        &open_recognition_capture,
    )
}

fn coordinate_running_recognition_with_capture<R: Runtime>(
    app: &AppHandle<R>,
    config: &AppConfig,
    publisher: Option<&RuntimeChatboxPublisher>,
    generation: &RuntimeGeneration,
    recognition: &mut RunningRecognition,
    open_capture: &dyn Fn(&crate::config::AudioConfig) -> AppResult<Box<dyn RecognitionCapture>>,
) -> AppResult<()> {
    let mut active_capture = None;
    let mut audio_level_revision = 0_u64;
    let mut audio_sequence = 0_u64;
    let coordinator = RecognitionCoordinator {
        app,
        config,
        publisher,
        generation,
        open_capture,
    };

    loop {
        if generation.is_work_cancelled() || generation.is_hard_stop_requested() {
            return Ok(());
        }

        loop {
            let signal = match recognition.signals.try_recv() {
                Ok(signal) => signal,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return finish_unexpected_recognition_owner(
                        coordinator.app,
                        coordinator.publisher,
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
        }

        let Some(active) = active_capture.as_mut() else {
            let signal = match recognition.signals.recv_timeout(RECEIVE_TIMEOUT) {
                Ok(signal) => signal,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return finish_unexpected_recognition_owner(
                        coordinator.app,
                        coordinator.publisher,
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
                        timestamp_ms: now_ms(),
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
            captured_at_ms: now_ms(),
            sample_rate_hz: active.capture.sample_rate(),
            samples: samples.into_boxed_slice(),
        }) {
            Ok(()) => {}
            Err(RecognitionSubmitError::Backpressure) => {
                return Err(AppError::stt_backpressure(
                    "The recognition backend could not keep up with microphone audio; the bounded recognition audio budget filled, so the session stopped instead of silently dropping audio.",
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
        let config = self.config;
        let publisher = self.publisher;
        let generation = self.generation;
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

                let capture = open_capture(&config.audio)?;
                let sample_rate = capture.sample_rate();
                let audio_level = speech_gate_level_meter(sample_rate)?;
                if !recognition.is_accepting_audio() {
                    drop(capture);
                    return Ok(RecognitionCoordinatorFlow::Continue);
                }

                let running = generation.commit_if_active(|| {
                record_and_emit_runtime_status(
                    app,
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
                            DiagnosticCategory::Stt,
                            "stt.reconnected",
                            "Recognition connection restored",
                            "Microphone capture resumed with a fresh provider session. No prior audio was replayed.",
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
                attempt,
                delay_ms,
            } => {
                drop(active_capture.take());
                generation.abort_active_units_for_reconnect(app, publisher)?;
                let reconnecting = generation.commit_if_active(|| {
                record_and_emit_runtime_status(
                    app,
                    RuntimeStatus::Reconnecting,
                    Some(format!(
                        "Recognition connection interrupted; retry {attempt} in {delay_ms} ms"
                    )),
                );
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::warning(
                        DiagnosticCategory::Stt,
                        "stt.reconnecting",
                        "Recognition connection interrupted",
                        format!(
                            "Microphone capture is paused; retry {attempt} begins in {delay_ms} ms. Unconfirmed speech was discarded."
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
                let outcome = generation.submit_recognition_event(app, publisher, event)?;
                Ok(if outcome == RecognitionEventSubmitOutcome::Stopped {
                    RecognitionCoordinatorFlow::Stopped
                } else {
                    RecognitionCoordinatorFlow::Continue
                })
            }
        }
    }
}

fn finish_unexpected_recognition_owner<R: Runtime>(
    app: &AppHandle<R>,
    publisher: Option<&RuntimeChatboxPublisher>,
    generation: &RuntimeGeneration,
    recognition: &mut RunningRecognition,
    active_capture: &mut Option<ActiveRecognitionCapture>,
) -> AppResult<()> {
    drop(active_capture.take());
    let owner_error = recognition.stop().err().unwrap_or_else(|| {
        AppError::runtime(
            "Recognition session owner stopped unexpectedly while the runtime was active.",
        )
    });
    if let Err(cleanup_error) = generation.abort_active_units_for_terminal_failure(app, publisher) {
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
#[path = "runtime_tests.rs"]
mod tests;
