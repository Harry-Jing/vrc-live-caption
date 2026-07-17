//! Runtime lifecycle for Phase 1 outgoing captions.
//!
//! The capture loop drains microphone samples and never performs blocking STT
//! upload work. Completed speech segments are sent to a bounded STT worker queue;
//! per-segment STT or OSC failures emit diagnostics and keep the runtime alive.
//! Startup failures such as invalid config or unavailable microphone remain fatal.
//!
//! Every utterance announced with `utterance-started` resolves with either a
//! final transcript or an `utterance-ended` event, so the UI never waits on a
//! transcript that cannot arrive. Transcript events carry recognition text
//! only; listening indicators are derived from lifecycle events in the UI.
//!
//! Stop is a hard cutoff: the microphone is released within one receive timeout,
//! buffered and queued speech is discarded instead of drained, and no App or
//! Chatbox caption text is committed after the stop request. A state-clearing
//! typing-off packet is sent before waiting for an STT request that is already
//! in flight, so runtime commands must run off the main thread
//! (`#[tauri::command(async)]`) to keep the window responsive during that wait.

use crate::audio::{open_input_capture, receive_audio};
use crate::chatbox_pacer::ChatboxPacer;
use crate::chatbox_publisher::{
    CompletedChatboxPublisher, CompletedPublisherEvent, PublisherCloseReason, PublisherDiagnostic,
    PublisherReporter, PublisherSubmitOutcome,
};
use crate::config::{AppConfig, OscConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, TranscriptUpdate, UtteranceEndReason,
    emit_diagnostic, emit_status, emit_transcript_final, emit_utterance_ended,
    emit_utterance_started, next_utterance_id,
};
use crate::osc::ChatboxOscSender;
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeCredentialSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase,
    RuntimeSessionSnapshot,
};
use crate::segmenter::SpeechSegmenter;
use crate::stt::{build_stt_client, transcribe_openai_wav};
use reqwest::blocking::Client;
use secrecy::SecretString;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};

const RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
const STT_QUEUE_CAPACITY: usize = 2;
const SPEECH_RMS_THRESHOLD: f32 = 0.012;
const SILENCE_TIMEOUT: Duration = Duration::from_millis(1200);
// Voiced audio only; long enough to drop clicks and pops, short enough to
// keep one-word utterances such as "Yes".
const MIN_VOICED_SECONDS: f32 = 0.3;
// This is only the absolute fallback for uninterrupted speech; the 1.2-second
// silence boundary still closes normal utterances earlier. Phase 1 VRChat
// testing found that 12 seconds split an approximately 20-second thought even
// though both ordered units were preserved, so the bounded cloud path now uses
// 30 seconds. Keep this internal and re-measure latency before raising it again.
const MAX_SEGMENT_SECONDS: f32 = 30.0;
const PREROLL_SECONDS: f32 = 0.25;

fn new_phase_one_segmenter(sample_rate: u32) -> SpeechSegmenter {
    SpeechSegmenter::new(
        sample_rate,
        SPEECH_RMS_THRESHOLD,
        SILENCE_TIMEOUT,
        MIN_VOICED_SECONDS,
        MAX_SEGMENT_SECONDS,
        PREROLL_SECONDS,
    )
}

pub(crate) struct RuntimeManager {
    handle: Mutex<Option<RuntimeHandle>>,
    stop_epoch: AtomicU64,
}

pub(crate) struct RuntimeStartRequest {
    pub(crate) config: AppConfig,
    pub(crate) chatbox_pacer: ChatboxPacer,
    pub(crate) generation_id: u64,
    pub(crate) config_revision: u64,
    pub(crate) openai_api_key: Option<SecretString>,
    pub(crate) credential: Option<RuntimeCredentialSnapshot>,
    pub(crate) expected_stop_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStartOutcome {
    Started,
    SupersededByStop,
}

struct RuntimeHandle {
    generation: RuntimeGeneration,
    publisher: Option<CompletedChatboxPublisher>,
    join_handle: JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct RuntimeGeneration {
    output_gate: Arc<Mutex<()>>,
    hard_stop_requested: Arc<AtomicBool>,
    work_cancelled: Arc<AtomicBool>,
}

struct SpeechSegment {
    utterance_id: String,
    sample_rate: u32,
    samples: Vec<f32>,
}

struct NoFinalUtteranceResolution {
    utterance_id: String,
    reason: UtteranceEndReason,
}

enum RuntimePublisherInit {
    Disabled,
    Ready(CompletedChatboxPublisher),
    Unavailable(AppError),
}

impl RuntimeGeneration {
    pub(crate) fn active() -> Self {
        Self {
            output_gate: Arc::new(Mutex::new(())),
            hard_stop_requested: Arc::new(AtomicBool::new(false)),
            work_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn request_stop(
        &self,
        publisher: Option<&CompletedChatboxPublisher>,
    ) -> AppResult<()> {
        // Cancel capture and provider work before waiting for either sink.
        // The explicit marker also prevents a new commit from overtaking Stop
        // while an earlier App emit still owns the output gate.
        self.hard_stop_requested.store(true, Ordering::SeqCst);
        self.cancel_work();

        // Keep the App-output gate closed while the Chatbox gate is closed so
        // Stop has one linearizable cutoff across both sinks. A commit that
        // validated before the explicit marker belongs before Stop; every
        // later commit is rejected by this generation forever.
        self.close_publisher_at_boundary(publisher, PublisherCloseReason::Stop)
    }

    fn close_publisher_at_boundary(
        &self,
        publisher: Option<&CompletedChatboxPublisher>,
        reason: PublisherCloseReason,
    ) -> AppResult<()> {
        // Close admission before waiting on an older App/Chatbox commit. This
        // keeps late STT results non-blocking while making them observe a
        // closed Publisher immediately. An already-linearized transport call
        // may finish; the gate below waits for it before returning.
        let close_result = match publisher {
            Some(publisher) => publisher.request_close(reason),
            None => Ok(()),
        };

        // Recover the guard even after a panic poisoned the gate. Normal
        // commits already fail closed on poison, but Publisher admission must
        // still close so Stop cannot hang while joining an idle worker.
        let (_output_gate, gate_was_poisoned) = match self.output_gate.lock() {
            Ok(gate) => (gate, false),
            Err(poisoned) => (poisoned.into_inner(), true),
        };

        if gate_was_poisoned {
            let close_note = match close_result {
                Ok(()) => " Publisher shutdown was still requested.".to_string(),
                Err(error) => format!(" Publisher shutdown also failed: {error}"),
            };
            Err(AppError::state(format!(
                "Runtime generation lock was poisoned.{close_note}"
            )))
        } else {
            close_result
        }
    }

    pub(crate) fn commit_if_active(&self, commit: impl FnOnce()) -> AppResult<bool> {
        let _output_gate = self
            .output_gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation lock was poisoned."))?;

        if self.hard_stop_requested.load(Ordering::SeqCst) {
            return Ok(false);
        }

        commit();
        Ok(true)
    }

    fn cancel_work(&self) {
        self.work_cancelled.store(true, Ordering::SeqCst);
    }

    fn is_work_cancelled(&self) -> bool {
        self.work_cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn is_hard_stopped(&self) -> bool {
        self.hard_stop_requested.load(Ordering::SeqCst)
    }

    fn try_begin_work(&self) -> bool {
        // These loads are the work-submission decision point. If they win the
        // race with Stop, the request is in flight and may finish, but its
        // result still has to pass commit_if_active.
        !self.is_work_cancelled() && !self.is_hard_stopped()
    }

    fn work_cancelled(&self) -> &AtomicBool {
        &self.work_cancelled
    }
}

impl Default for RuntimeManager {
    fn default() -> Self {
        Self {
            handle: Mutex::new(None),
            stop_epoch: AtomicU64::new(0),
        }
    }
}

impl RuntimeManager {
    pub(crate) fn stop_epoch(&self) -> u64 {
        self.stop_epoch.load(Ordering::SeqCst)
    }

    pub(crate) fn start_epoch_is_current(&self, expected_stop_epoch: u64) -> bool {
        self.stop_epoch() == expected_stop_epoch
    }

    pub(crate) fn ensure_start_available<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(app, &mut guard)?;

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        Ok(())
    }

    pub(crate) fn start<F>(
        &self,
        app: AppHandle,
        request: RuntimeStartRequest,
        install_session: F,
    ) -> AppResult<RuntimeStartOutcome>
    where
        F: FnOnce(RuntimeSessionSnapshot) -> AppResult<()>,
    {
        let RuntimeStartRequest {
            config,
            chatbox_pacer,
            generation_id,
            config_revision,
            openai_api_key,
            credential,
            expected_stop_epoch,
        } = request;
        config.validate()?;

        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(&app, &mut guard)?;

        // Stop increments its epoch before waiting for this handle lock. The
        // comparison and handle installation are therefore one linearized
        // decision: an earlier Start cannot come back to life after Stop has
        // already returned while Start was resolving slow desired-state I/O.
        if !self.start_epoch_is_current(expected_stop_epoch) {
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        let generation = RuntimeGeneration::active();
        let publisher_init =
            initialize_runtime_publisher(&app, &config.osc, chatbox_pacer, generation.clone());
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
            credential,
            chatbox,
            uploads_microphone_audio: matches!(config.stt.provider, SttProvider::OpenAi),
        };
        if let Err(error) = install_session(session) {
            let _ = generation.request_stop(publisher.as_ref());
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
                )
            })
            .map_err(|error| AppError::runtime(format!("Failed to start runtime thread: {error}")));
        let join_handle = match join_handle {
            Ok(join_handle) => join_handle,
            Err(error) => {
                let _ = generation.request_stop(publisher.as_ref());
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
            emit_status(
                app,
                RuntimeStatus::Stopped,
                Some("Runtime is already stopped".to_string()),
            );
            return Ok(());
        };

        if let Err(error) = handle.generation.request_stop(handle.publisher.as_ref()) {
            handle.generation.cancel_work();
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Completed publisher could not close"),
            );
        }
        emit_status(
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
                DiagnosticUpdate::from_error(&error, "Completed publisher failed while stopping"),
            );
        }

        if runtime_panicked {
            let error = AppError::runtime("Runtime thread panicked while stopping.");
            emit_status(app, RuntimeStatus::Error, Some(error.to_string()));
            return Err(error);
        }

        emit_status(
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
                DiagnosticUpdate::from_error(&error, "Completed publisher could not close"),
            );
        }
        if let Err(error) = publisher.join() {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Completed publisher failed while closing"),
            );
        }
    }

    handle
        .join_handle
        .join()
        .map_err(|_| AppError::runtime("Runtime thread panicked after stopping."))
}

fn initialize_runtime_publisher(
    app: &AppHandle,
    config: &OscConfig,
    chatbox_pacer: ChatboxPacer,
    generation: RuntimeGeneration,
) -> RuntimePublisherInit {
    if !config.enabled {
        return RuntimePublisherInit::Disabled;
    }

    let sender = match ChatboxOscSender::new(config) {
        Ok(sender) => sender,
        Err(error) => return RuntimePublisherInit::Unavailable(error),
    };
    let reporter_app = app.clone();
    let reporter: PublisherReporter = Arc::new(move |diagnostic| {
        emit_publisher_diagnostic(&reporter_app, diagnostic);
    });

    match CompletedChatboxPublisher::start(Arc::new(sender), chatbox_pacer, generation, reporter) {
        Ok(publisher) => RuntimePublisherInit::Ready(publisher),
        Err(error) => RuntimePublisherInit::Unavailable(error),
    }
}

fn run_runtime_thread(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    publisher: Option<CompletedChatboxPublisher>,
    generation: RuntimeGeneration,
) {
    let error_generation = generation.clone();
    let cleanup_publisher = publisher.clone();
    let runtime_app = app.clone();

    supervise_runtime_thread(
        &app,
        &error_generation,
        cleanup_publisher.as_ref(),
        move || run_runtime(runtime_app, config, openai_api_key, publisher, generation),
    );
}

fn supervise_runtime_thread<R: Runtime>(
    app: &AppHandle<R>,
    generation: &RuntimeGeneration,
    publisher: Option<&CompletedChatboxPublisher>,
    run: impl FnOnce() -> AppResult<()>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    let runtime_result = match outcome {
        Ok(runtime_result) => runtime_result,
        Err(panic) => {
            finish_runtime_output(app, generation, publisher, PublisherCloseReason::Stop);
            tracing::error!("runtime thread panicked; its generation and Publisher were stopped");
            emit_status(
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

    let reason = if generation.is_hard_stopped() {
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

        if generation.is_hard_stopped() {
            return;
        }

        emit_status(app, RuntimeStatus::Error, Some(error.to_string()));
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Runtime stopped with an error"),
        );
    }
}

fn finish_runtime_output<R: Runtime>(
    app: &AppHandle<R>,
    generation: &RuntimeGeneration,
    publisher: Option<&CompletedChatboxPublisher>,
    reason: PublisherCloseReason,
) {
    let close_result = match reason {
        PublisherCloseReason::Stop => generation.request_stop(publisher),
        PublisherCloseReason::RuntimeError => match publisher {
            Some(publisher) => generation
                .close_publisher_at_boundary(Some(publisher), PublisherCloseReason::RuntimeError),
            None => Ok(()),
        },
    };
    if let Err(error) = close_result {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Completed publisher could not close"),
        );
    }

    if let Some(publisher) = publisher
        && let Err(error) = publisher.join()
    {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Completed publisher failed while closing"),
        );
    }
}

fn run_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    publisher: Option<CompletedChatboxPublisher>,
    generation: RuntimeGeneration,
) -> AppResult<()> {
    let started = generation.commit_if_active(|| {
        emit_status(
            &app,
            RuntimeStatus::Starting,
            Some("Starting outgoing caption runtime".to_string()),
        );
    })?;
    if !started {
        return Ok(());
    }

    match config.stt.provider {
        SttProvider::Mock => run_mock_runtime(app, generation),
        SttProvider::OpenAi => {
            let api_key = openai_api_key.ok_or_else(|| {
                AppError::secret("OpenAI API key was not loaded before runtime startup.")
            })?;

            run_openai_runtime(app, config, api_key, publisher, generation)
        }
    }
}

fn run_mock_runtime<R: Runtime>(app: AppHandle<R>, generation: RuntimeGeneration) -> AppResult<()> {
    let running = generation.commit_if_active(|| {
        emit_status(
            &app,
            RuntimeStatus::Running,
            Some("Mock runtime is running".to_string()),
        );
        emit_diagnostic(
            &app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Runtime,
                "runtime.mock_started",
                "Mock runtime started",
                "Use Mock Transcript to test normalized runtime events.",
            ),
        );
    })?;
    if !running {
        return Ok(());
    }

    while !generation.is_work_cancelled() {
        thread::sleep(RECEIVE_TIMEOUT);
    }

    Ok(())
}

fn run_openai_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<CompletedChatboxPublisher>,
    generation: RuntimeGeneration,
) -> AppResult<()> {
    if !generation.try_begin_work() {
        return Ok(());
    }

    let capture = open_input_capture(&config.audio)?;
    if generation.is_hard_stopped() {
        return Ok(());
    }

    let sample_rate = capture.sample_rate;
    let running = generation.commit_if_active(|| {
        emit_status(
            &app,
            RuntimeStatus::Running,
            Some("Listening for microphone speech".to_string()),
        );
        emit_diagnostic(
            &app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Audio,
                "audio.capture_started",
                "Microphone capture started",
                format!("Capturing mono audio at {sample_rate} Hz."),
            ),
        );
    })?;
    if !running {
        return Ok(());
    }

    let (segment_sender, segment_receiver) = sync_channel(STT_QUEUE_CAPACITY);
    // Created once per runtime and reused by the worker across segments so the
    // HTTP client keeps its connection pool.
    let http_client = build_stt_client()?;
    let stt_worker = spawn_stt_worker(
        app.clone(),
        config.clone(),
        openai_api_key,
        http_client,
        publisher.clone(),
        segment_receiver,
        generation.clone(),
    )?;
    let mut segmenter = new_phase_one_segmenter(sample_rate);
    let mut utterance_id: Option<String> = None;
    let stream = capture.stream;

    // Once the worker exists, no capture error may return before shutdown has
    // set the shared stop flag and joined that worker. Keeping the fallible loop
    // inside this result makes every exit converge on the cleanup below.
    let capture_result = (|| -> AppResult<()> {
        while !generation.is_work_cancelled() {
            let Some(samples) = receive_audio(&capture.receiver, RECEIVE_TIMEOUT)? else {
                if let Some(samples) = segmenter.tick(Instant::now()) {
                    queue_speech_segment(
                        &app,
                        segmenter.sample_rate(),
                        samples,
                        &mut utterance_id,
                        &segment_sender,
                        publisher.as_ref(),
                    )?;
                }
                continue;
            };

            let update = segmenter.push_samples(samples, Instant::now());

            if update.speech_started {
                let next_utterance = next_utterance_id("speech");
                emit_utterance_started(&app, next_utterance.clone());
                start_chatbox_activity(&app, publisher.as_ref(), &next_utterance);
                utterance_id = Some(next_utterance);
            }

            if let Some(samples) = update.ready_segment {
                queue_speech_segment(
                    &app,
                    segmenter.sample_rate(),
                    samples,
                    &mut utterance_id,
                    &segment_sender,
                    publisher.as_ref(),
                )?;
            }
        }

        Ok(())
    })();

    // Close Chatbox output before releasing anything that can take time. An
    // in-flight transcription may finish concurrently with stream teardown.
    generation.cancel_work();
    if let Some(publisher) = &publisher {
        let reason = if generation.is_hard_stopped() {
            PublisherCloseReason::Stop
        } else {
            PublisherCloseReason::RuntimeError
        };
        if let Err(error) = generation.close_publisher_at_boundary(Some(publisher), reason) {
            emit_diagnostic(
                &app,
                DiagnosticUpdate::from_error(&error, "Completed publisher could not close"),
            );
        }
    }
    // Shutdown path: release the microphone before joining either worker, and
    // discard buffered tail speech instead of sending it to STT after capture
    // has ended.
    drop(stream);
    if let Some(publisher) = &publisher
        && let Err(error) = publisher.join()
    {
        emit_diagnostic(
            &app,
            DiagnosticUpdate::from_error(&error, "Completed publisher failed while closing"),
        );
    }
    let worker_result = finish_stt_worker_after_capture(
        capture_result,
        generation.work_cancelled(),
        segment_sender,
        stt_worker,
    );
    let tail_speech_discarded = segmenter.finish().is_some();

    if tail_speech_discarded {
        if let Some(utterance_id) = utterance_id {
            end_utterance_without_final(
                &app,
                publisher.as_ref(),
                utterance_id,
                UtteranceEndReason::Discarded,
            );
        }

        emit_diagnostic(
            &app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.tail_speech_discarded",
                "Unsent speech discarded",
                "Speech buffered when microphone capture ended was discarded without transcription.",
            ),
        );
    }

    worker_result
}

fn finish_stt_worker_after_capture(
    capture_result: AppResult<()>,
    stop_requested: &AtomicBool,
    segment_sender: SyncSender<SpeechSegment>,
    stt_worker: JoinHandle<()>,
) -> AppResult<()> {
    stop_requested.store(true, Ordering::Relaxed);
    drop(segment_sender);

    let join_result = stt_worker
        .join()
        .map_err(|_| AppError::runtime("STT worker thread panicked while stopping."));

    match (capture_result, join_result) {
        (Err(capture_error), Err(join_error)) => {
            tracing::warn!(
                code = join_error.code(),
                error_message = %join_error,
                "STT worker also failed while closing after a capture error"
            );
            Err(capture_error)
        }
        (Err(capture_error), Ok(())) => Err(capture_error),
        (Ok(()), Err(join_error)) => Err(join_error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn queue_speech_segment(
    app: &AppHandle,
    sample_rate: u32,
    samples: Vec<f32>,
    utterance_id: &mut Option<String>,
    segment_sender: &SyncSender<SpeechSegment>,
    publisher: Option<&CompletedChatboxPublisher>,
) -> AppResult<()> {
    // The segmenter only yields segments that reached the voiced minimum, and
    // crossing the voiced minimum announces the utterance first, so an id is
    // always present here. A missing id means segmentation broke that
    // invariant, and silently minting a fresh id would send the UI a
    // transcript for an utterance that was never announced.
    let utterance_id = utterance_id.take().ok_or_else(|| {
        AppError::runtime("Speech segment was ready without an announced utterance.")
    })?;

    match segment_sender.try_send(SpeechSegment {
        utterance_id,
        sample_rate,
        samples,
    }) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(segment)) => {
            emit_diagnostic(
                app,
                DiagnosticUpdate::warning(
                    DiagnosticCategory::Stt,
                    "stt.segment_dropped",
                    "Speech segment dropped",
                    format!(
                        "STT is still processing earlier audio, so {:.1} seconds of captured speech was skipped.",
                        segment.samples.len() as f32 / segment.sample_rate as f32
                    ),
                ),
            );
            end_utterance_without_final(
                app,
                publisher,
                segment.utterance_id,
                UtteranceEndReason::Discarded,
            );

            Ok(())
        }
        Err(TrySendError::Disconnected(segment)) => {
            end_utterance_without_final(
                app,
                publisher,
                segment.utterance_id,
                UtteranceEndReason::Discarded,
            );

            Err(AppError::runtime(
                "STT worker stopped unexpectedly while the runtime was still capturing audio.",
            ))
        }
    }
}

fn spawn_stt_worker(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    http_client: Client,
    publisher: Option<CompletedChatboxPublisher>,
    segment_receiver: Receiver<SpeechSegment>,
    generation: RuntimeGeneration,
) -> AppResult<JoinHandle<()>> {
    thread::Builder::new()
        .name("vrc-live-caption-stt-worker".to_string())
        .spawn(move || {
            run_stt_worker(
                app,
                config,
                openai_api_key,
                http_client,
                publisher,
                segment_receiver,
                generation,
                |client, config, api_key, sample_rate, samples| {
                    transcribe_openai_wav(client, &config.stt, api_key, sample_rate, samples)
                },
            )
        })
        .map_err(|error| AppError::runtime(format!("Failed to start STT worker thread: {error}")))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Worker lifecycle dependencies stay explicit; the final argument is the test seam for cloud transcription."
)]
fn run_stt_worker<R: Runtime>(
    app: AppHandle<R>,
    config: AppConfig,
    openai_api_key: SecretString,
    http_client: Client,
    publisher: Option<CompletedChatboxPublisher>,
    segment_receiver: Receiver<SpeechSegment>,
    generation: RuntimeGeneration,
    transcribe: impl Fn(&Client, &AppConfig, &SecretString, u32, &[f32]) -> AppResult<String>,
) {
    let mut discarded_segments: usize = 0;
    while let Ok(segment) = segment_receiver.recv() {
        if generation.is_work_cancelled() {
            discarded_segments += 1;
            end_utterance_without_final(
                &app,
                publisher.as_ref(),
                segment.utterance_id,
                UtteranceEndReason::Discarded,
            );
            continue;
        }

        if let Err(error) = transcribe_and_emit_final(
            &app,
            &config,
            &openai_api_key,
            &http_client,
            segment,
            publisher.as_ref(),
            &generation,
            &transcribe,
        ) {
            tracing::warn!(
                code = error.code(),
                error_message = %error,
                "speech segment failed"
            );

            emit_diagnostic(
                &app,
                DiagnosticUpdate::from_error(&error, "Speech segment failed"),
            );
        }
    }

    if discarded_segments > 0 {
        tracing::info!(discarded_segments, "discarded queued speech on stop");

        emit_diagnostic(
            &app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.queued_speech_discarded",
                "Queued speech discarded",
                format!(
                    "Discarded {discarded_segments} speech segment(s) that were still waiting for STT when the runtime stopped."
                ),
            ),
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The transcript operation keeps its runtime sinks explicit; the final argument is the test seam for cloud transcription."
)]
fn transcribe_and_emit_final<R: Runtime>(
    app: &AppHandle<R>,
    config: &AppConfig,
    openai_api_key: &SecretString,
    http_client: &Client,
    segment: SpeechSegment,
    publisher: Option<&CompletedChatboxPublisher>,
    generation: &RuntimeGeneration,
    transcribe: &impl Fn(&Client, &AppConfig, &SecretString, u32, &[f32]) -> AppResult<String>,
) -> AppResult<()> {
    if !generation.try_begin_work() {
        end_utterance_without_final(
            app,
            publisher,
            segment.utterance_id,
            UtteranceEndReason::Discarded,
        );
        emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.segment_discarded_on_stop",
                "Speech segment discarded",
                "Runtime stop was requested before this segment entered transcription.",
            ),
        );

        return Ok(());
    }

    emit_diagnostic(
        app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Stt,
            "stt.segment_started",
            "Sending speech segment to STT",
            format!(
                "Captured {:.1} seconds for final transcription.",
                segment.samples.len() as f32 / segment.sample_rate as f32
            ),
        ),
    );

    let utterance_id = segment.utterance_id.clone();
    let text = match transcribe(
        http_client,
        config,
        openai_api_key,
        segment.sample_rate,
        &segment.samples,
    ) {
        Ok(text) => text,
        Err(error) => {
            let committed = generation.commit_if_active(|| {
                // Resolve the utterance for the UI; the caller reports error details.
                end_utterance_without_final(
                    app,
                    publisher,
                    utterance_id.clone(),
                    UtteranceEndReason::SttFailed,
                );
            })?;
            if !committed {
                discard_late_transcription_result(app, publisher, utterance_id);
                return Ok(());
            }

            return Err(error);
        }
    };

    if text.is_empty() {
        let committed = generation.commit_if_active(|| {
            end_utterance_without_final(
                app,
                publisher,
                utterance_id.clone(),
                UtteranceEndReason::NoSpeech,
            );
            emit_diagnostic(
                app,
                DiagnosticUpdate::info(
                    DiagnosticCategory::Stt,
                    "stt.no_speech",
                    "STT returned no speech",
                    "The captured segment did not contain recognized words.",
                ),
            );
        })?;
        if !committed {
            discard_late_transcription_result(app, publisher, utterance_id);
        }

        return Ok(());
    }

    let committed = generation.commit_if_active(|| {
        emit_transcript_final(
            app,
            TranscriptUpdate {
                utterance_id: utterance_id.clone(),
                text: text.clone(),
                language: config.stt.language.clone(),
                provider: config.stt.provider.as_str().to_string(),
                revision: 1,
            },
        );
    })?;

    if !committed {
        discard_late_transcription_result(app, publisher, utterance_id);
        return Ok(());
    }

    // Hard Stop rejects the Chatbox candidate immediately. A non-Stop runtime
    // failure closes the publisher separately; the outcome below makes that
    // race visible without turning the STT worker into a Chatbox waiter.
    if generation.is_hard_stopped() {
        emit_chatbox_send_skipped_on_stop(app);
        abort_chatbox_activity(app, publisher, &utterance_id);

        return Ok(());
    }

    let Some(publisher) = publisher else {
        let (code, message, detail) = if config.osc.enabled {
            (
                "osc.output_unavailable",
                "Chatbox output unavailable",
                "OSC output could not be initialized when the runtime started.",
            )
        } else {
            (
                "osc.output_disabled",
                "Chatbox output skipped",
                "OSC output is disabled in settings.",
            )
        };
        emit_diagnostic(
            app,
            DiagnosticUpdate::info(DiagnosticCategory::Osc, code, message, detail),
        );

        return Ok(());
    };

    match publisher.try_submit(CompletedPublisherEvent::Completed {
        unit_id: utterance_id,
        text,
    }) {
        Ok(PublisherSubmitOutcome::Handled) => {}
        Ok(PublisherSubmitOutcome::Closed) => {
            if generation.is_hard_stopped() {
                emit_chatbox_send_skipped_on_stop(app);
            } else {
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::info(
                        DiagnosticCategory::Osc,
                        "osc.completed_unit_discarded_after_close",
                        "Completed Chatbox publication discarded",
                        "The runtime output worker closed before this completed caption could enter its queue. The App transcript remains available.",
                    ),
                );
            }
        }
        Err(error) => emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Completed Chatbox publication was rejected"),
        ),
    }

    Ok(())
}

fn discard_late_transcription_result<R: Runtime>(
    app: &AppHandle<R>,
    publisher: Option<&CompletedChatboxPublisher>,
    utterance_id: String,
) {
    end_utterance_without_final(app, publisher, utterance_id, UtteranceEndReason::Discarded);
    emit_diagnostic(
        app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Stt,
            "stt.result_discarded_on_stop",
            "Late transcription discarded",
            "The transcription completed after its runtime generation was stopped.",
        ),
    );
}

fn emit_chatbox_send_skipped_on_stop<R: Runtime>(app: &AppHandle<R>) {
    emit_diagnostic(
        app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.send_skipped_on_stop",
            "Chatbox send skipped",
            "Runtime stop was requested before this transcript could be sent.",
        ),
    );
}

fn emit_publisher_diagnostic<R: Runtime>(app: &AppHandle<R>, diagnostic: PublisherDiagnostic) {
    let update = match diagnostic {
        PublisherDiagnostic::UnitPublished {
            unit_id,
            page_count,
            byte_count,
            target,
        } => DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.completed_unit_sent",
            "Completed caption published",
            format!(
                "Published {page_count} ordered page(s) for {unit_id} to {target} using {byte_count} encoded byte(s)."
            ),
        ),
        PublisherDiagnostic::UnitDroppedOverload {
            unit_id,
            page_count,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_dropped_overload",
            "Completed caption dropped from Chatbox backlog",
            format!(
                "Dropped the oldest unstarted caption unit {unit_id} as one complete {page_count}-page publication because the Chatbox backlog was full. The App transcript remains available."
            ),
        ),
        PublisherDiagnostic::UnitRejectedOverload {
            unit_id,
            page_count,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_rejected_overload",
            "Completed caption could not enter the Chatbox backlog",
            format!(
                "Rejected caption unit {unit_id} as one complete {page_count}-page publication because it could not fit safely within the bounded Chatbox backlog. No partial pages were queued; the App transcript remains available."
            ),
        ),
        PublisherDiagnostic::UnitExpired {
            unit_id,
            page_count,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_expired",
            "Completed caption expired from Chatbox backlog",
            format!(
                "Discarded unstarted caption unit {unit_id} as one complete {page_count}-page publication after it exceeded the provisional backlog age. The App transcript remains available."
            ),
        ),
        PublisherDiagnostic::LayoutFailed { unit_id, reason } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_layout_failed",
            "Completed caption could not be laid out for Chatbox",
            format!("Caption unit {unit_id} was not published: {reason}"),
        ),
        PublisherDiagnostic::UnitSendFailed {
            unit_id,
            page_index,
            page_count,
            pages_sent,
            error,
        } => DiagnosticUpdate::from_error(
            &error,
            format!(
                "Completed Chatbox publication failed for {unit_id} on page {page_index} of {page_count} after {pages_sent} successful page(s); the failed page was not retried and the unit's remaining pages were discarded"
            ),
        ),
        PublisherDiagnostic::PagesDiscardedOnClose {
            reason,
            unit_count,
            page_count,
            started_unit_count,
        } => {
            let (code, message) = match reason {
                PublisherCloseReason::Stop => (
                    "osc.completed_pages_discarded_on_stop",
                    "Pending Chatbox captions discarded on Stop",
                ),
                PublisherCloseReason::RuntimeError => (
                    "osc.completed_pages_discarded_on_error",
                    "Pending Chatbox captions discarded after Runtime failure",
                ),
            };
            DiagnosticUpdate::info(
                DiagnosticCategory::Osc,
                code,
                message,
                format!(
                    "Discarded {page_count} unsent page(s) across {unit_count} caption unit(s), including {started_unit_count} unit(s) whose publication had begun."
                ),
            )
        }
        PublisherDiagnostic::TypingFailed { is_typing, error } => {
            let transition = if is_typing { "on" } else { "off" };
            DiagnosticUpdate::from_error(
                &error,
                format!("Chatbox typing indicator could not turn {transition}"),
            )
        }
        PublisherDiagnostic::WorkerFailed { reason } => DiagnosticUpdate::error(
            DiagnosticCategory::Osc,
            "osc.completed_publisher_failed",
            "Completed Chatbox publisher stopped unexpectedly",
            reason,
        ),
    };

    emit_diagnostic(app, update);
}

fn start_chatbox_activity(
    app: &AppHandle,
    publisher: Option<&CompletedChatboxPublisher>,
    utterance_id: &str,
) {
    let Some(publisher) = publisher else {
        return;
    };

    if let Err(error) = publisher.try_submit(CompletedPublisherEvent::Started {
        unit_id: utterance_id.to_string(),
    }) {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Chatbox activity could not start"),
        );
    }
}

fn end_utterance_without_final<R: Runtime>(
    app: &AppHandle<R>,
    publisher: Option<&CompletedChatboxPublisher>,
    utterance_id: String,
    reason: UtteranceEndReason,
) {
    let resolution = NoFinalUtteranceResolution {
        utterance_id,
        reason,
    };

    if let Err(error) =
        complete_no_final_utterance(publisher, resolution, |utterance_id, reason| {
            emit_utterance_ended(app, utterance_id, reason);
        })
    {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Chatbox activity could not resolve"),
        );
    }
}

fn complete_no_final_utterance(
    publisher: Option<&CompletedChatboxPublisher>,
    resolution: NoFinalUtteranceResolution,
    emit_ended: impl FnOnce(String, UtteranceEndReason),
) -> AppResult<()> {
    let utterance_id = resolution.utterance_id.clone();
    emit_ended(resolution.utterance_id, resolution.reason);

    let Some(publisher) = publisher else {
        return Ok(());
    };

    publisher
        .try_submit(CompletedPublisherEvent::Aborted {
            unit_id: utterance_id,
        })
        .map(|_| ())
}

fn abort_chatbox_activity<R: Runtime>(
    app: &AppHandle<R>,
    publisher: Option<&CompletedChatboxPublisher>,
    utterance_id: &str,
) {
    let Some(publisher) = publisher else {
        return;
    };

    if let Err(error) = publisher.try_submit(CompletedPublisherEvent::Aborted {
        unit_id: utterance_id.to_string(),
    }) {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Chatbox activity could not resolve"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatbox_publisher::{ChatboxSendReceipt, ChatboxTransport};
    use tauri::Listener;

    #[test]
    fn phase_one_segmenter_keeps_twenty_seconds_whole_until_silence() {
        let sample_rate = 10;
        let mut segmenter = new_phase_one_segmenter(sample_rate);
        let started_at = Instant::now();

        for sample_index in 0_u64..200 {
            let update = segmenter.push_samples(
                vec![0.2],
                started_at + Duration::from_millis(sample_index * 100),
            );
            assert!(update.ready_segment.is_none());
        }

        assert_eq!(
            segmenter.tick(started_at + Duration::from_millis(21_100)),
            Some(vec![0.2; 200])
        );
    }

    #[test]
    fn phase_one_segmenter_continues_without_loss_after_the_thirty_second_limit() {
        let mut segmenter = new_phase_one_segmenter(10);
        let started_at = Instant::now();
        let mut speech_starts = 0;
        let mut ready_segments = Vec::new();

        for sample_index in 0_u64..400 {
            let update = segmenter.push_samples(
                vec![0.2],
                started_at + Duration::from_millis(sample_index * 100),
            );
            speech_starts += usize::from(update.speech_started);
            if let Some(samples) = update.ready_segment {
                ready_segments.push(samples);
            }
        }

        if let Some(samples) = segmenter.finish() {
            ready_segments.push(samples);
        }

        assert_eq!(speech_starts, 2);
        assert_eq!(
            ready_segments.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![300, 100]
        );
        assert_eq!(ready_segments.iter().map(Vec::len).sum::<usize>(), 400);
    }

    #[test]
    fn runtime_manager_closes_the_generation_before_joining_the_worker() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let generation = RuntimeGeneration::active();
        let manager = Arc::new(RuntimeManager::default());
        let (worker_ready_sender, worker_ready_receiver) = std::sync::mpsc::channel();
        let (release_worker_sender, release_worker_receiver) = std::sync::mpsc::channel();
        let join_handle = thread::spawn(move || {
            let _ = worker_ready_sender.send(());
            let _ = release_worker_receiver.recv();
        });

        {
            let mut handle = manager
                .handle
                .lock()
                .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
            *handle = Some(RuntimeHandle {
                generation: generation.clone(),
                publisher: None,
                join_handle,
            });
        }
        worker_ready_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Runtime test worker did not start."))?;

        let stop_manager = Arc::clone(&manager);
        let stop_app = app.handle().clone();
        let (stop_started_sender, stop_started_receiver) = std::sync::mpsc::channel();
        let stop = thread::spawn(move || {
            let _ = stop_started_sender.send(());
            stop_manager.stop(&stop_app)
        });
        stop_started_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Runtime stop test thread did not start."))?;

        let deadline = Instant::now() + Duration::from_secs(1);
        let generation_closed_before_join = loop {
            if generation.is_hard_stopped() && !generation.commit_if_active(|| {})? {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(1));
        };

        release_worker_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the runtime test worker."))?;
        stop.join()
            .map_err(|_| AppError::runtime("Runtime stop test thread panicked."))??;
        assert!(generation_closed_before_join);

        Ok(())
    }

    #[test]
    fn finished_error_handle_is_reaped_before_a_restart_availability_check() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let manager = RuntimeManager::default();
        let (finished_sender, finished_receiver) = std::sync::mpsc::channel();
        let join_handle = thread::spawn(move || {
            let _ = finished_sender.send(());
        });
        finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Finished runtime test thread did not exit."))?;
        let deadline = Instant::now() + Duration::from_secs(1);
        while !join_handle.is_finished() {
            if Instant::now() >= deadline {
                return Err(AppError::runtime(
                    "Finished runtime test thread did not become joinable.",
                ));
            }
            thread::yield_now();
        }
        {
            let mut handle = manager
                .handle
                .lock()
                .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
            *handle = Some(RuntimeHandle {
                generation: RuntimeGeneration::active(),
                publisher: None,
                join_handle,
            });
        }

        manager.ensure_start_available(app.handle())?;
        let handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        assert!(handle.is_none());
        Ok(())
    }

    #[test]
    fn stop_invalidates_an_uncommitted_start_epoch() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let manager = RuntimeManager::default();
        let expected_stop_epoch = manager.stop_epoch();

        assert!(manager.start_epoch_is_current(expected_stop_epoch));
        manager.stop(app.handle())?;
        assert!(!manager.start_epoch_is_current(expected_stop_epoch));
        Ok(())
    }

    #[test]
    fn stop_cancels_work_before_waiting_for_an_app_commit() -> AppResult<()> {
        let generation = RuntimeGeneration::active();
        let commit_generation = generation.clone();
        let stop_generation = generation.clone();
        let (commit_started_sender, commit_started_receiver) = std::sync::mpsc::channel();
        let (release_commit_sender, release_commit_receiver) = std::sync::mpsc::channel();

        let commit = thread::spawn(move || {
            commit_generation.commit_if_active(|| {
                let _ = commit_started_sender.send(());
                let _ = release_commit_receiver.recv();
            })
        });
        commit_started_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("App commit did not reach the test boundary."))?;

        let stop = thread::spawn(move || stop_generation.request_stop(None));
        let deadline = Instant::now() + Duration::from_secs(1);
        let cancelled_before_commit_finished = loop {
            if generation.is_work_cancelled() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(1));
        };

        release_commit_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the App commit."))?;
        assert!(
            commit
                .join()
                .map_err(|_| AppError::runtime("App commit test thread panicked."))??
        );
        stop.join()
            .map_err(|_| AppError::runtime("Runtime stop test thread panicked."))??;
        assert!(cancelled_before_commit_finished);
        assert!(!generation.commit_if_active(|| {})?);

        Ok(())
    }

    #[test]
    fn poisoned_generation_gate_still_closes_and_joins_the_publisher() -> AppResult<()> {
        let generation = RuntimeGeneration::active();
        let output_gate = Arc::clone(&generation.output_gate);
        let poisoner = thread::spawn(move || {
            if let Ok(_gate) = output_gate.lock() {
                std::panic::resume_unwind(Box::new("poison generation gate for shutdown coverage"));
            }
        });
        assert!(poisoner.join().is_err());
        let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;

        assert!(generation.request_stop(Some(&publisher)).is_err());
        publisher.join()?;
        assert_eq!(
            publisher.try_submit(CompletedPublisherEvent::Completed {
                unit_id: "late".to_string(),
                text: "late".to_string(),
            })?,
            PublisherSubmitOutcome::Closed
        );
        assert!(matches!(
            text_receiver.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));

        Ok(())
    }

    #[test]
    fn runtime_thread_panic_invalidates_generation_and_closes_publisher() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });
        let generation = RuntimeGeneration::active();
        let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;
        let panic_app = app.handle().clone();
        let panic_generation = generation.clone();
        let panic_publisher = publisher.clone();

        let panicking_runtime = thread::spawn(move || {
            supervise_runtime_thread(
                &panic_app,
                &panic_generation,
                Some(&panic_publisher),
                || -> AppResult<()> {
                    std::panic::resume_unwind(Box::new(
                        "panic runtime thread for supervisor coverage",
                    ));
                },
            );
        });
        assert!(panicking_runtime.join().is_err());

        assert!(generation.is_hard_stopped());
        assert!(!generation.commit_if_active(|| {})?);
        publisher.join()?;
        assert_eq!(
            publisher.try_submit(CompletedPublisherEvent::Completed {
                unit_id: "late-after-panic".to_string(),
                text: "late".to_string(),
            })?,
            PublisherSubmitOutcome::Closed
        );
        assert!(matches!(
            text_receiver.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        let diagnostic = receive_json_event(&diagnostic_receiver, "Runtime panic diagnostic")?;
        assert_eq!(diagnostic["code"], "runtime.thread_panicked");

        Ok(())
    }

    #[test]
    fn stopped_generation_does_not_begin_provider_work() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
        app.listen("utterance-ended", move |event| {
            let _ = ended_sender.send(event.payload().to_string());
        });
        let generation = RuntimeGeneration::active();
        generation.request_stop(None)?;
        let provider_called = Arc::new(AtomicBool::new(false));
        let provider_called_by_worker = Arc::clone(&provider_called);
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        transcribe_and_emit_final(
            app.handle(),
            &config,
            &SecretString::from("test-key".to_string()),
            &http_client,
            test_speech_segment("not-submitted"),
            None,
            &generation,
            &move |_client, _config, _api_key, _sample_rate, _samples| {
                provider_called_by_worker.store(true, Ordering::Relaxed);
                Ok("must not be returned".to_string())
            },
        )?;

        assert!(!provider_called.load(Ordering::Relaxed));
        let ended_event = receive_json_event(&ended_receiver, "discarded unsubmitted utterance")?;
        assert_eq!(ended_event["utteranceId"], "not-submitted");
        assert_eq!(ended_event["reason"], "discarded");

        Ok(())
    }

    #[test]
    fn late_empty_and_error_results_are_discarded_after_stop() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        app.listen("utterance-ended", move |event| {
            let _ = ended_sender.send(event.payload().to_string());
        });
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        for (utterance_id, return_error) in [("late-empty", false), ("late-error", true)] {
            let generation = RuntimeGeneration::active();
            let provider_generation = generation.clone();
            transcribe_and_emit_final(
                app.handle(),
                &config,
                &SecretString::from("test-key".to_string()),
                &http_client,
                test_speech_segment(utterance_id),
                None,
                &generation,
                &move |_client, _config, _api_key, _sample_rate, _samples| {
                    provider_generation.request_stop(None)?;
                    if return_error {
                        Err(AppError::stt("Late provider failure."))
                    } else {
                        Ok(String::new())
                    }
                },
            )?;
        }

        for utterance_id in ["late-empty", "late-error"] {
            let ended_event = receive_json_event(&ended_receiver, "discarded late utterance")?;
            assert_eq!(ended_event["utteranceId"], utterance_id);
            assert_eq!(ended_event["reason"], "discarded");
        }

        let diagnostic_codes = diagnostic_receiver
            .try_iter()
            .map(|payload| {
                serde_json::from_str::<serde_json::Value>(&payload)
                    .map(|event| event["code"].as_str().unwrap_or_default().to_string())
                    .map_err(|error| {
                        AppError::runtime(format!("Failed to parse a diagnostic event: {error}"))
                    })
            })
            .collect::<AppResult<Vec<_>>>()?;
        assert_eq!(
            diagnostic_codes
                .iter()
                .filter(|code| code.as_str() == "stt.result_discarded_on_stop")
                .count(),
            2
        );
        assert!(!diagnostic_codes.iter().any(|code| code == "stt.no_speech"));

        Ok(())
    }

    #[test]
    fn stop_between_starting_and_mock_runtime_blocks_late_running() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let (status_sender, status_receiver) = std::sync::mpsc::channel();
        app.listen("runtime-status", move |event| {
            let _ = status_sender.send(event.payload().to_string());
        });
        let generation = RuntimeGeneration::active();

        assert!(generation.commit_if_active(|| {
            emit_status(
                app.handle(),
                RuntimeStatus::Starting,
                Some("Starting test runtime".to_string()),
            );
        })?);
        generation.request_stop(None)?;
        run_mock_runtime(app.handle().clone(), generation)?;

        let starting_event = receive_json_event(&status_receiver, "starting runtime status")?;
        assert_eq!(starting_event["status"], "starting");
        assert!(matches!(
            status_receiver.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));

        Ok(())
    }

    #[test]
    fn stopped_generation_cannot_publish_while_a_new_generation_can() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let (final_sender, final_receiver) = std::sync::mpsc::channel();
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
        app.listen("transcript-final", move |event| {
            let _ = final_sender.send(event.payload().to_string());
        });
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });
        app.listen("utterance-ended", move |event| {
            let _ = ended_sender.send(event.payload().to_string());
        });

        let generation = RuntimeGeneration::active();
        let (stopped_publisher, stopped_text_receiver) =
            runtime_test_publisher(generation.clone())?;
        assert_eq!(
            stopped_publisher.try_submit(CompletedPublisherEvent::Started {
                unit_id: "stopped-in-flight".to_string(),
            })?,
            PublisherSubmitOutcome::Handled
        );
        let worker_generation = generation.clone();
        let worker_publisher = stopped_publisher.clone();
        let (in_flight_sender, in_flight_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        let worker = thread::spawn(move || {
            transcribe_and_emit_final(
                &app_handle,
                &config,
                &SecretString::from("test-key".to_string()),
                &http_client,
                test_speech_segment("stopped-in-flight"),
                Some(&worker_publisher),
                &worker_generation,
                &move |_client, _config, _api_key, _sample_rate, _samples| {
                    in_flight_sender.send(()).map_err(|_| {
                        AppError::runtime("Could not announce the in-flight test segment.")
                    })?;
                    release_receiver.recv().map_err(|_| {
                        AppError::runtime("Could not release the in-flight test segment.")
                    })?;

                    Ok("late final".to_string())
                },
            )
        });

        in_flight_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("STT test worker did not start its segment."))?;
        generation.request_stop(Some(&stopped_publisher))?;
        stopped_publisher.join()?;

        let current_app_handle = app.handle().clone();
        let current_generation = RuntimeGeneration::active();
        let (current_publisher, current_text_receiver) =
            runtime_test_publisher(current_generation.clone())?;
        let current_config = AppConfig::default();
        let current_http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        assert_eq!(
            current_publisher.try_submit(CompletedPublisherEvent::Started {
                unit_id: "current".to_string(),
            })?,
            PublisherSubmitOutcome::Handled
        );
        transcribe_and_emit_final(
            &current_app_handle,
            &current_config,
            &SecretString::from("test-key".to_string()),
            &current_http_client,
            test_speech_segment("current"),
            Some(&current_publisher),
            &current_generation,
            &|_client, _config, _api_key, _sample_rate, _samples| Ok("current final".to_string()),
        )?;
        let current_text = current_text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Current publisher did not send its caption."))?;
        assert_eq!(current_text, "current final");
        current_publisher.request_close(PublisherCloseReason::RuntimeError)?;
        current_publisher.join()?;

        release_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the STT test worker."))?;
        worker
            .join()
            .map_err(|_| AppError::runtime("STT test worker panicked."))??;

        let final_event = receive_json_event(&final_receiver, "current final transcript")?;
        assert_eq!(final_event["utteranceId"], "current");
        assert_eq!(final_event["text"], "current final");
        assert!(matches!(
            final_receiver.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        let ended_event = receive_json_event(&ended_receiver, "discarded old utterance")?;
        assert_eq!(ended_event["utteranceId"], "stopped-in-flight");
        assert_eq!(ended_event["reason"], "discarded");
        assert!(matches!(
            stopped_text_receiver.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));

        let mut late_result_discarded = false;
        for payload in diagnostic_receiver.try_iter() {
            let event = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
                AppError::runtime(format!("Failed to parse a diagnostic event: {error}"))
            })?;
            late_result_discarded |= event["code"] == "stt.result_discarded_on_stop";
        }
        assert!(late_result_discarded);

        Ok(())
    }

    #[test]
    fn runtime_error_close_preserves_an_in_flight_app_final_but_rejects_chatbox() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let (final_sender, final_receiver) = std::sync::mpsc::channel();
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        app.listen("transcript-final", move |event| {
            let _ = final_sender.send(event.payload().to_string());
        });
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });

        let generation = RuntimeGeneration::active();
        let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;
        assert_eq!(
            publisher.try_submit(CompletedPublisherEvent::Started {
                unit_id: "in-flight-error".to_string(),
            })?,
            PublisherSubmitOutcome::Handled
        );
        let worker_generation = generation.clone();
        let worker_publisher = publisher.clone();
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;
        let worker = thread::spawn(move || {
            transcribe_and_emit_final(
                &app_handle,
                &config,
                &SecretString::from("test-key".to_string()),
                &http_client,
                test_speech_segment("in-flight-error"),
                Some(&worker_publisher),
                &worker_generation,
                &move |_client, _config, _api_key, _sample_rate, _samples| {
                    entered_sender.send(()).map_err(|_| {
                        AppError::runtime("Could not announce the in-flight test request.")
                    })?;
                    release_receiver.recv().map_err(|_| {
                        AppError::runtime("Could not release the in-flight test request.")
                    })?;
                    Ok("preserved in App".to_string())
                },
            )
        });

        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("In-flight test request did not start."))?;
        generation.cancel_work();
        generation
            .close_publisher_at_boundary(Some(&publisher), PublisherCloseReason::RuntimeError)?;
        publisher.join()?;
        release_sender
            .send(())
            .map_err(|_| AppError::runtime("Could not release the in-flight test request."))?;
        worker
            .join()
            .map_err(|_| AppError::runtime("In-flight test worker panicked."))??;

        let final_event = receive_json_event(&final_receiver, "in-flight final transcript")?;
        assert_eq!(final_event["utteranceId"], "in-flight-error");
        assert_eq!(final_event["text"], "preserved in App");
        assert!(matches!(
            text_receiver.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        let diagnostic_codes = (0..2)
            .map(|_| {
                receive_json_event(&diagnostic_receiver, "in-flight Runtime error diagnostic")
                    .map(|event| event["code"].as_str().unwrap_or_default().to_string())
            })
            .collect::<AppResult<Vec<_>>>()?;
        assert!(
            diagnostic_codes
                .iter()
                .any(|code| code == "osc.completed_unit_discarded_after_close")
        );

        Ok(())
    }

    #[test]
    fn capture_error_preserves_in_flight_app_final_and_discards_queued_speech() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let (final_sender, final_receiver) = std::sync::mpsc::channel();
        let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
        app.listen("transcript-final", move |event| {
            let _ = final_sender.send(event.payload().to_string());
        });
        app.listen("utterance-ended", move |event| {
            let _ = ended_sender.send(event.payload().to_string());
        });

        let generation = RuntimeGeneration::active();
        let (segment_sender, segment_receiver) = sync_channel(STT_QUEUE_CAPACITY);
        let (in_flight_sender, in_flight_receiver) = std::sync::mpsc::channel();
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        segment_sender
            .send(test_speech_segment("in-flight"))
            .map_err(|_| AppError::runtime("Failed to queue the in-flight test segment."))?;
        segment_sender
            .send(test_speech_segment("queued"))
            .map_err(|_| AppError::runtime("Failed to queue the pending test segment."))?;

        let worker_generation = generation.clone();
        let worker = thread::spawn(move || {
            let transcribe_generation = worker_generation.clone();
            run_stt_worker(
                app_handle,
                config,
                SecretString::from("test-key".to_string()),
                http_client,
                None,
                segment_receiver,
                worker_generation,
                move |_client, _config, _api_key, _sample_rate, _samples| {
                    in_flight_sender.send(()).map_err(|_| {
                        AppError::runtime("Could not announce the in-flight test segment.")
                    })?;
                    while !transcribe_generation.is_work_cancelled() {
                        thread::yield_now();
                    }

                    Ok("in-flight final".to_string())
                },
            );
        });

        in_flight_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("STT test worker did not start its first segment."))?;

        let shutdown = finish_stt_worker_after_capture(
            Err(AppError::audio("Microphone capture disconnected.")),
            generation.work_cancelled(),
            segment_sender,
            worker,
        );

        let error = shutdown
            .err()
            .ok_or_else(|| AppError::runtime("Capture failure was not returned after cleanup."))?;
        assert_eq!(error.code(), "audio.failed");

        let final_event = receive_json_event(&final_receiver, "final transcript")?;
        assert_eq!(final_event["utteranceId"], "in-flight");
        assert_eq!(final_event["text"], "in-flight final");

        let ended_event = receive_json_event(&ended_receiver, "discarded utterance")?;
        assert_eq!(ended_event["utteranceId"], "queued");
        assert_eq!(ended_event["reason"], "discarded");

        Ok(())
    }

    #[test]
    fn publisher_diagnostics_keep_stable_osc_wire_codes() -> AppResult<()> {
        let app = tauri::test::mock_app();
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });
        let diagnostics = vec![
            (
                PublisherDiagnostic::UnitPublished {
                    unit_id: "published".to_string(),
                    page_count: 2,
                    byte_count: 42,
                    target: "127.0.0.1:9000".to_string(),
                },
                "osc.completed_unit_sent",
                "info",
            ),
            (
                PublisherDiagnostic::UnitDroppedOverload {
                    unit_id: "dropped".to_string(),
                    page_count: 2,
                },
                "osc.completed_unit_dropped_overload",
                "warning",
            ),
            (
                PublisherDiagnostic::UnitRejectedOverload {
                    unit_id: "rejected".to_string(),
                    page_count: 33,
                },
                "osc.completed_unit_rejected_overload",
                "warning",
            ),
            (
                PublisherDiagnostic::UnitExpired {
                    unit_id: "expired".to_string(),
                    page_count: 2,
                },
                "osc.completed_unit_expired",
                "warning",
            ),
            (
                PublisherDiagnostic::LayoutFailed {
                    unit_id: "layout".to_string(),
                    reason: "test layout failure".to_string(),
                },
                "osc.completed_layout_failed",
                "warning",
            ),
            (
                PublisherDiagnostic::UnitSendFailed {
                    unit_id: "send".to_string(),
                    page_index: 2,
                    page_count: 3,
                    pages_sent: 1,
                    error: AppError::osc_send("test", "send failure".to_string()),
                },
                "osc.send_failed",
                "error",
            ),
            (
                PublisherDiagnostic::PagesDiscardedOnClose {
                    reason: PublisherCloseReason::Stop,
                    unit_count: 2,
                    page_count: 3,
                    started_unit_count: 1,
                },
                "osc.completed_pages_discarded_on_stop",
                "info",
            ),
            (
                PublisherDiagnostic::PagesDiscardedOnClose {
                    reason: PublisherCloseReason::RuntimeError,
                    unit_count: 2,
                    page_count: 3,
                    started_unit_count: 1,
                },
                "osc.completed_pages_discarded_on_error",
                "info",
            ),
            (
                PublisherDiagnostic::TypingFailed {
                    is_typing: false,
                    error: AppError::osc_send("test", "typing failure".to_string()),
                },
                "osc.send_failed",
                "error",
            ),
            (
                PublisherDiagnostic::WorkerFailed {
                    reason: "worker failure".to_string(),
                },
                "osc.completed_publisher_failed",
                "error",
            ),
        ];

        for (diagnostic, expected_code, expected_severity) in diagnostics {
            emit_publisher_diagnostic(app.handle(), diagnostic);
            let event = receive_json_event(&diagnostic_receiver, "Publisher diagnostic")?;
            assert_eq!(event["category"], "osc");
            assert_eq!(event["code"], expected_code);
            assert_eq!(event["severity"], expected_severity);
            if expected_code == "osc.completed_unit_rejected_overload" {
                assert!(
                    event["detail"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("No partial pages were queued")
                );
            }
            if expected_code == "osc.completed_pages_discarded_on_stop" {
                assert!(
                    event["detail"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("Discarded 3 unsent page(s)")
                );
            }
        }

        Ok(())
    }

    #[test]
    fn every_no_final_resolution_emits_app_lifecycle_and_turns_typing_off() -> AppResult<()> {
        let reasons = [
            UtteranceEndReason::NoSpeech,
            UtteranceEndReason::SttFailed,
            UtteranceEndReason::Discarded,
        ];

        for (index, reason) in reasons.into_iter().enumerate() {
            let utterance_id = format!("no-final-{index}");
            let (text_sender, text_receiver) = std::sync::mpsc::channel();
            let (typing_sender, typing_receiver) = std::sync::mpsc::channel();
            let publisher = CompletedChatboxPublisher::start(
                Arc::new(RecordingChatboxTransport {
                    text_sender,
                    typing_sender: Some(typing_sender),
                }),
                ChatboxPacer::default(),
                RuntimeGeneration::active(),
                Arc::new(|_| {}),
            )?;
            assert_eq!(
                publisher.try_submit(CompletedPublisherEvent::Started {
                    unit_id: utterance_id.clone(),
                })?,
                PublisherSubmitOutcome::Handled
            );
            assert!(
                typing_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| AppError::runtime("No-final typing indicator did not turn on."))?
            );
            let resolution = NoFinalUtteranceResolution {
                utterance_id: utterance_id.clone(),
                reason,
            };
            let mut emitted = None;

            complete_no_final_utterance(
                Some(&publisher),
                resolution,
                |emitted_utterance_id, emitted_reason| {
                    emitted = Some((emitted_utterance_id, emitted_reason));
                },
            )?;

            let (emitted_utterance_id, emitted_reason) = emitted
                .ok_or_else(|| AppError::runtime("No-final completion event was not emitted."))?;
            assert_eq!(emitted_utterance_id, utterance_id);
            assert!(same_utterance_end_reason(emitted_reason, reason));
            assert!(
                !typing_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| AppError::runtime(
                        "No-final typing indicator did not turn off."
                    ))?
            );
            assert!(matches!(
                text_receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            publisher.request_close(PublisherCloseReason::RuntimeError)?;
            publisher.join()?;
        }

        Ok(())
    }

    fn same_utterance_end_reason(left: UtteranceEndReason, right: UtteranceEndReason) -> bool {
        matches!(
            (left, right),
            (UtteranceEndReason::NoSpeech, UtteranceEndReason::NoSpeech)
                | (UtteranceEndReason::SttFailed, UtteranceEndReason::SttFailed)
                | (UtteranceEndReason::Discarded, UtteranceEndReason::Discarded)
        )
    }

    struct RecordingChatboxTransport {
        text_sender: std::sync::mpsc::Sender<String>,
        typing_sender: Option<std::sync::mpsc::Sender<bool>>,
    }

    impl ChatboxTransport for RecordingChatboxTransport {
        fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
            self.text_sender.send(text.to_string()).map_err(|_| {
                AppError::osc_send(
                    "runtime test transport",
                    "Text receiver disconnected.".to_string(),
                )
            })?;

            Ok(ChatboxSendReceipt {
                target: "runtime-test".to_string(),
                byte_count: text.len(),
            })
        }

        fn send_typing(&self, is_typing: bool) -> AppResult<()> {
            if let Some(sender) = &self.typing_sender {
                sender.send(is_typing).map_err(|_| {
                    AppError::osc_send(
                        "runtime test transport",
                        "Typing receiver disconnected.".to_string(),
                    )
                })?;
            }
            Ok(())
        }
    }

    fn runtime_test_publisher(
        generation: RuntimeGeneration,
    ) -> AppResult<(CompletedChatboxPublisher, std::sync::mpsc::Receiver<String>)> {
        let (text_sender, text_receiver) = std::sync::mpsc::channel();
        let reporter: PublisherReporter = Arc::new(|_| {});
        let publisher = CompletedChatboxPublisher::start(
            Arc::new(RecordingChatboxTransport {
                text_sender,
                typing_sender: None,
            }),
            ChatboxPacer::default(),
            generation,
            reporter,
        )?;

        Ok((publisher, text_receiver))
    }

    fn test_speech_segment(utterance_id: &str) -> SpeechSegment {
        SpeechSegment {
            utterance_id: utterance_id.to_string(),
            sample_rate: 16_000,
            samples: vec![0.0; 160],
        }
    }

    fn receive_json_event(
        receiver: &std::sync::mpsc::Receiver<String>,
        event_name: &str,
    ) -> AppResult<serde_json::Value> {
        let payload = receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
            AppError::runtime(format!("Did not receive the expected {event_name} event."))
        })?;

        serde_json::from_str(&payload).map_err(|error| {
            AppError::runtime(format!("Failed to parse the {event_name} event: {error}"))
        })
    }
}
