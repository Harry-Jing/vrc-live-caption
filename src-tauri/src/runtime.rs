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
use crate::config::{AppConfig, OscConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, RuntimeStatusEvent, TranscriptUpdate,
    UtteranceEndReason, emit_diagnostic, emit_status, emit_transcript_final, emit_utterance_ended,
    emit_utterance_started, next_utterance_id,
};
use crate::osc::{ChatboxActivityHandle, ChatboxOscSender};
use crate::secrets::openai_api_key as load_openai_api_key;
use crate::segmenter::SpeechSegmenter;
use crate::stt::{build_stt_client, transcribe_openai_wav};
use reqwest::blocking::Client;
use secrecy::SecretString;
use std::sync::atomic::{AtomicBool, Ordering};
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
const MAX_SEGMENT_SECONDS: f32 = 12.0;
const PREROLL_SECONDS: f32 = 0.25;

pub(crate) struct RuntimeManager {
    handle: Mutex<Option<RuntimeHandle>>,
    status: Mutex<RuntimeStatusEvent>,
}

struct RuntimeHandle {
    generation: RuntimeGeneration,
    chatbox_activity: Option<ChatboxActivityHandle>,
    join_handle: JoinHandle<()>,
}

#[derive(Clone)]
struct RuntimeGeneration {
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

enum RuntimeChatboxInit {
    Disabled,
    Ready(ChatboxOscSender),
    Unavailable(AppError),
}

impl RuntimeGeneration {
    fn active() -> Self {
        Self {
            output_gate: Arc::new(Mutex::new(())),
            hard_stop_requested: Arc::new(AtomicBool::new(false)),
            work_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn request_stop(&self, activity: Option<&ChatboxActivityHandle>) -> AppResult<()> {
        // Cancel capture and provider work before waiting for either sink.
        // The explicit marker also prevents a new commit from overtaking Stop
        // while an earlier App emit still owns the output gate.
        self.hard_stop_requested.store(true, Ordering::SeqCst);
        self.cancel_work();

        // Keep the App-output gate closed while the Chatbox gate is closed so
        // Stop has one linearizable cutoff across both sinks. A commit that
        // validated before the explicit marker belongs before Stop; every
        // later commit is rejected by this generation forever.
        let _output_gate = self
            .output_gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation lock was poisoned."))?;

        match activity {
            Some(activity) => activity.request_stop(&self.work_cancelled),
            None => Ok(()),
        }
    }

    fn commit_if_active(&self, commit: impl FnOnce()) -> AppResult<bool> {
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

    fn is_hard_stopped(&self) -> bool {
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
            status: Mutex::new(RuntimeStatusEvent::idle()),
        }
    }
}

impl RuntimeManager {
    pub(crate) fn status_snapshot(&self) -> AppResult<RuntimeStatusEvent> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| AppError::state("Runtime status lock was poisoned."))
    }

    pub(crate) fn replace_status(&self, status: RuntimeStatusEvent) -> AppResult<()> {
        let mut guard = self
            .status
            .lock()
            .map_err(|_| AppError::state("Runtime status lock was poisoned."))?;
        *guard = status;

        Ok(())
    }

    pub(crate) fn start(
        &self,
        app: AppHandle,
        config: AppConfig,
        chatbox_pacer: ChatboxPacer,
    ) -> AppResult<()> {
        config.validate()?;

        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(&app, &mut guard)?;

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        let openai_api_key = if matches!(config.stt.provider, SttProvider::OpenAi) {
            match load_openai_api_key() {
                Ok(api_key) => Some(api_key),
                Err(error) => {
                    emit_diagnostic(
                        &app,
                        DiagnosticUpdate::error(
                            DiagnosticCategory::Config,
                            "config.openai_api_key_missing",
                            "Cloud STT is not configured",
                            error.to_string(),
                        ),
                    );

                    return Err(error);
                }
            }
        } else {
            None
        };
        let osc_sender = match initialize_runtime_chatbox(&config.osc, chatbox_pacer) {
            RuntimeChatboxInit::Disabled => None,
            RuntimeChatboxInit::Ready(sender) => Some(sender),
            RuntimeChatboxInit::Unavailable(error) => {
                emit_chatbox_activity_failure(&app, &error, "Chatbox OSC output could not start");
                None
            }
        };
        let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

        let generation = RuntimeGeneration::active();
        let thread_generation = generation.clone();
        let join_handle = thread::Builder::new()
            .name("vrc-live-caption-runtime".to_string())
            .spawn(move || {
                run_runtime_thread(app, config, openai_api_key, osc_sender, thread_generation)
            })
            .map_err(|error| {
                AppError::runtime(format!("Failed to start runtime thread: {error}"))
            })?;

        *guard = Some(RuntimeHandle {
            generation,
            chatbox_activity,
            join_handle,
        });

        Ok(())
    }

    pub(crate) fn stop<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
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

        if let Err(error) = handle
            .generation
            .request_stop(handle.chatbox_activity.as_ref())
        {
            handle.generation.cancel_work();
            emit_chatbox_activity_failure(app, &error, "Typing indicator cleanup failed");
        }
        emit_status(
            app,
            RuntimeStatus::Stopping,
            Some("Stopping runtime and discarding pending speech".to_string()),
        );

        let runtime_panicked = handle.join_handle.join().is_err();

        if let Some(activity) = &handle.chatbox_activity
            && let Err(cleanup_error) = activity.finish_stop()
        {
            emit_chatbox_activity_failure(
                app,
                &cleanup_error,
                "Typing indicator cleanup after runtime stop failed",
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

fn clear_finished_runtime(app: &AppHandle, handle: &mut Option<RuntimeHandle>) -> AppResult<()> {
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

    if let Some(activity) = &handle.chatbox_activity
        && let Err(cleanup_error) = activity.finish_stop()
    {
        emit_chatbox_activity_failure(
            app,
            &cleanup_error,
            "Typing indicator cleanup before runtime restart failed",
        );
    }

    handle
        .join_handle
        .join()
        .map_err(|_| AppError::runtime("Runtime thread panicked after stopping."))
}

fn initialize_runtime_chatbox(
    config: &OscConfig,
    chatbox_pacer: ChatboxPacer,
) -> RuntimeChatboxInit {
    if !config.enabled {
        return RuntimeChatboxInit::Disabled;
    }

    match ChatboxOscSender::new(config, chatbox_pacer) {
        Ok(sender) => RuntimeChatboxInit::Ready(sender),
        Err(error) => RuntimeChatboxInit::Unavailable(error),
    }
}

fn run_runtime_thread(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    osc_sender: Option<ChatboxOscSender>,
    generation: RuntimeGeneration,
) {
    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);
    let error_generation = generation.clone();

    if let Err(error) = run_runtime(app.clone(), config, openai_api_key, osc_sender, generation) {
        if let Some(activity) = &chatbox_activity
            && let Err(cleanup_error) = activity.finish_after_error()
        {
            emit_chatbox_activity_failure(
                &app,
                &cleanup_error,
                "Typing indicator cleanup after runtime failure failed",
            );
        }

        tracing::warn!(
            code = error.code(),
            error_message = %error,
            "runtime stopped with error"
        );

        if error_generation.is_hard_stopped() {
            return;
        }

        emit_status(&app, RuntimeStatus::Error, Some(error.to_string()));
        emit_diagnostic(
            &app,
            DiagnosticUpdate::from_error(&error, "Runtime stopped with an error"),
        );
    }
}

fn run_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    osc_sender: Option<ChatboxOscSender>,
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

    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

    match config.stt.provider {
        SttProvider::Mock => run_mock_runtime(app, generation, chatbox_activity.as_ref()),
        SttProvider::OpenAi => {
            let api_key = openai_api_key.ok_or_else(|| {
                AppError::secret("OpenAI API key was not loaded before runtime startup.")
            })?;

            run_openai_runtime(app, config, api_key, osc_sender, generation)
        }
    }
}

fn run_mock_runtime<R: Runtime>(
    app: AppHandle<R>,
    generation: RuntimeGeneration,
    chatbox_activity: Option<&ChatboxActivityHandle>,
) -> AppResult<()> {
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

    finish_chatbox_stop(&app, chatbox_activity);

    Ok(())
}

fn run_openai_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    osc_sender: Option<ChatboxOscSender>,
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
    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);
    let stt_worker = spawn_stt_worker(
        app.clone(),
        config.clone(),
        openai_api_key,
        http_client,
        osc_sender,
        segment_receiver,
        generation.clone(),
    )?;
    let mut segmenter = SpeechSegmenter::new(
        sample_rate,
        SPEECH_RMS_THRESHOLD,
        SILENCE_TIMEOUT,
        MIN_VOICED_SECONDS,
        MAX_SEGMENT_SECONDS,
        PREROLL_SECONDS,
    );
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
                        chatbox_activity.as_ref(),
                    )?;
                }
                continue;
            };

            let update = segmenter.push_samples(samples, Instant::now());

            if update.speech_started {
                let next_utterance = next_utterance_id("speech");
                emit_utterance_started(&app, next_utterance.clone());
                start_chatbox_activity(&app, chatbox_activity.as_ref(), &next_utterance);
                utterance_id = Some(next_utterance);
            }

            if let Some(samples) = update.ready_segment {
                queue_speech_segment(
                    &app,
                    segmenter.sample_rate(),
                    samples,
                    &mut utterance_id,
                    &segment_sender,
                    chatbox_activity.as_ref(),
                )?;
            }
        }

        Ok(())
    })();

    let capture_failed = capture_result.is_err();
    // Close Chatbox output before releasing anything that can take time. An
    // in-flight transcription may finish concurrently with stream teardown.
    generation.cancel_work();
    // Stop path: release the microphone before waiting on the worker, and
    // discard buffered tail speech instead of sending it to STT after stop.
    drop(stream);
    let worker_result = if capture_failed {
        // The outer runtime error path retains responsibility for typing
        // cleanup through `finish_after_error`.
        finish_stt_worker_after_capture(
            capture_result,
            generation.work_cancelled(),
            segment_sender,
            stt_worker,
        )
    } else {
        let (typing_result, worker_result) =
            finish_chatbox_before_join(chatbox_activity.as_ref(), || {
                finish_stt_worker_after_capture(
                    capture_result,
                    generation.work_cancelled(),
                    segment_sender,
                    stt_worker,
                )
            });

        if let Err(error) = typing_result {
            emit_chatbox_activity_failure(&app, &error, "Typing indicator cleanup failed");
        }

        worker_result
    };
    let tail_speech_discarded = segmenter.finish().is_some();

    if tail_speech_discarded {
        if let Some(utterance_id) = utterance_id {
            end_utterance_without_final(
                &app,
                chatbox_activity.as_ref(),
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
                "Speech captured just before stop was discarded without transcription.",
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
    chatbox_activity: Option<&ChatboxActivityHandle>,
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
                chatbox_activity,
                segment.utterance_id,
                UtteranceEndReason::Discarded,
            );

            Ok(())
        }
        Err(TrySendError::Disconnected(segment)) => {
            end_utterance_without_final(
                app,
                chatbox_activity,
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
    osc_sender: Option<ChatboxOscSender>,
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
                osc_sender,
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
    mut osc_sender: Option<ChatboxOscSender>,
    segment_receiver: Receiver<SpeechSegment>,
    generation: RuntimeGeneration,
    transcribe: impl Fn(&Client, &AppConfig, &SecretString, u32, &[f32]) -> AppResult<String>,
) {
    let mut discarded_segments: usize = 0;
    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

    while let Ok(segment) = segment_receiver.recv() {
        if generation.is_work_cancelled() {
            discarded_segments += 1;
            end_utterance_without_final(
                &app,
                chatbox_activity.as_ref(),
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
            osc_sender.as_mut(),
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
    osc_sender: Option<&mut ChatboxOscSender>,
    generation: &RuntimeGeneration,
    transcribe: &impl Fn(&Client, &AppConfig, &SecretString, u32, &[f32]) -> AppResult<String>,
) -> AppResult<()> {
    let chatbox_activity = osc_sender.as_ref().map(|sender| sender.activity_handle());

    if !generation.try_begin_work() {
        end_utterance_without_final(
            app,
            chatbox_activity.as_ref(),
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
                    chatbox_activity.as_ref(),
                    utterance_id.clone(),
                    UtteranceEndReason::SttFailed,
                );
            })?;
            if !committed {
                discard_late_transcription_result(app, chatbox_activity.as_ref(), utterance_id);
                return Ok(());
            }

            return Err(error);
        }
    };

    if text.is_empty() {
        let committed = generation.commit_if_active(|| {
            end_utterance_without_final(
                app,
                chatbox_activity.as_ref(),
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
            discard_late_transcription_result(app, chatbox_activity.as_ref(), utterance_id);
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
        discard_late_transcription_result(app, chatbox_activity.as_ref(), utterance_id);
        return Ok(());
    }

    // A capture failure may cancel remaining worker activity without closing
    // the generation's App-output gate. Preserve its already in-flight App
    // result, but never send Chatbox text after any cancellation.
    if generation.is_work_cancelled() {
        emit_chatbox_send_skipped_on_stop(app);
        resolve_chatbox_activity(app, chatbox_activity.as_ref(), &utterance_id);

        return Ok(());
    }

    let Some(osc_sender) = osc_sender else {
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

    // The paced send also watches the stop flag itself: a stop requested
    // while the send is waiting out the pacing interval cancels the send
    // instead of flushing one more Chatbox message.
    let attempt = osc_sender.send_final_paced(&utterance_id, &text, generation.work_cancelled());

    if let Err(error) = attempt.typing {
        emit_chatbox_activity_failure(app, &error, "Typing indicator cleanup failed");
    }

    match attempt.text {
        Ok(Some(result)) => {
            let clipped_note = if result.clipped {
                " Text was clipped to fit the VRChat Chatbox layout."
            } else {
                ""
            };

            emit_diagnostic(
                app,
                DiagnosticUpdate::info(
                    DiagnosticCategory::Osc,
                    "osc.final_sent",
                    "Final transcript sent to Chatbox",
                    format!(
                        "Sent {} bytes to {}.{}",
                        result.byte_count, result.target, clipped_note
                    ),
                ),
            );

            Ok(())
        }
        Ok(None) => {
            emit_chatbox_send_skipped_on_stop(app);

            Ok(())
        }
        Err(error) => {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Chatbox output failed"),
            );

            Err(error)
        }
    }
}

fn discard_late_transcription_result<R: Runtime>(
    app: &AppHandle<R>,
    activity: Option<&ChatboxActivityHandle>,
    utterance_id: String,
) {
    end_utterance_without_final(app, activity, utterance_id, UtteranceEndReason::Discarded);
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

fn start_chatbox_activity(
    app: &AppHandle,
    activity: Option<&ChatboxActivityHandle>,
    utterance_id: &str,
) {
    let Some(activity) = activity else {
        return;
    };

    if let Err(error) = activity.utterance_started(utterance_id) {
        emit_chatbox_activity_failure(app, &error, "Typing indicator could not be enabled");
    }
}

fn end_utterance_without_final<R: Runtime>(
    app: &AppHandle<R>,
    activity: Option<&ChatboxActivityHandle>,
    utterance_id: String,
    reason: UtteranceEndReason,
) {
    let resolution = NoFinalUtteranceResolution {
        utterance_id,
        reason,
    };

    if let Err(error) = complete_no_final_utterance(activity, resolution, |utterance_id, reason| {
        emit_utterance_ended(app, utterance_id, reason);
    }) {
        emit_chatbox_activity_failure(app, &error, "Typing indicator cleanup failed");
    }
}

fn complete_no_final_utterance(
    activity: Option<&ChatboxActivityHandle>,
    resolution: NoFinalUtteranceResolution,
    emit_ended: impl FnOnce(String, UtteranceEndReason),
) -> AppResult<()> {
    let utterance_id = resolution.utterance_id.clone();
    emit_ended(resolution.utterance_id, resolution.reason);

    let Some(activity) = activity else {
        return Ok(());
    };

    activity.utterance_resolved(&utterance_id)
}

fn resolve_chatbox_activity<R: Runtime>(
    app: &AppHandle<R>,
    activity: Option<&ChatboxActivityHandle>,
    utterance_id: &str,
) {
    let Some(activity) = activity else {
        return;
    };

    if let Err(error) = activity.utterance_resolved(utterance_id) {
        emit_chatbox_activity_failure(app, &error, "Typing indicator cleanup failed");
    }
}

fn emit_chatbox_activity_failure<R: Runtime>(
    app: &AppHandle<R>,
    error: &AppError,
    message: &'static str,
) {
    tracing::warn!(
        code = error.code(),
        error_message = %error,
        "Chatbox typing indicator update failed"
    );
    emit_diagnostic(app, DiagnosticUpdate::from_error(error, message));
}

fn finish_chatbox_stop<R: Runtime>(app: &AppHandle<R>, activity: Option<&ChatboxActivityHandle>) {
    let Some(activity) = activity else {
        return;
    };

    if let Err(error) = activity.finish_stop() {
        emit_chatbox_activity_failure(app, &error, "Typing indicator cleanup failed");
    }
}

fn finish_chatbox_before_join<T>(
    activity: Option<&ChatboxActivityHandle>,
    join_worker: impl FnOnce() -> AppResult<T>,
) -> (AppResult<()>, AppResult<T>) {
    let typing_result = match activity {
        Some(activity) => activity.finish_stop(),
        None => Ok(()),
    };
    let worker_result = join_worker();

    (typing_result, worker_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscMessage, OscPacket, OscType, decoder};
    use std::net::UdpSocket;
    use tauri::Listener;

    #[test]
    fn disabled_osc_does_not_create_runtime_chatbox_output() {
        let config = crate::config::OscConfig {
            host: "does-not-resolve.invalid".to_string(),
            port: 9000,
            enabled: false,
        };

        assert!(matches!(
            initialize_runtime_chatbox(&config, ChatboxPacer::default()),
            RuntimeChatboxInit::Disabled
        ));
    }

    #[test]
    fn unavailable_osc_output_does_not_become_a_runtime_start_error() {
        let config = crate::config::OscConfig {
            host: "[::1]".to_string(),
            port: 9000,
            enabled: true,
        };

        assert!(matches!(
            initialize_runtime_chatbox(&config, ChatboxPacer::default()),
            RuntimeChatboxInit::Unavailable(_)
        ));
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
                chatbox_activity: None,
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
        run_mock_runtime(app.handle().clone(), generation, None)?;

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

        let (mut osc_sender, osc_receiver) = runtime_test_sender_and_receiver()?;
        let activity = osc_sender.activity_handle();
        let generation = RuntimeGeneration::active();
        let worker_generation = generation.clone();
        let (in_flight_sender, in_flight_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        activity.utterance_started("stopped-in-flight")?;
        assert_eq!(
            receive_runtime_test_packet(&osc_receiver)?,
            typing_packet(true)
        );

        let worker = thread::spawn(move || {
            transcribe_and_emit_final(
                &app_handle,
                &config,
                &SecretString::from("test-key".to_string()),
                &http_client,
                test_speech_segment("stopped-in-flight"),
                Some(&mut osc_sender),
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
        generation.request_stop(Some(&activity))?;
        activity.finish_stop()?;

        let current_app_handle = app.handle().clone();
        let (mut current_osc_sender, current_osc_receiver) = runtime_test_sender_and_receiver()?;
        let current_activity = current_osc_sender.activity_handle();
        let current_generation = RuntimeGeneration::active();
        let current_config = AppConfig::default();
        let current_http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        current_activity.utterance_started("current")?;
        assert_eq!(
            receive_runtime_test_packet(&current_osc_receiver)?,
            typing_packet(true)
        );
        transcribe_and_emit_final(
            &current_app_handle,
            &current_config,
            &SecretString::from("test-key".to_string()),
            &current_http_client,
            test_speech_segment("current"),
            Some(&mut current_osc_sender),
            &current_generation,
            &|_client, _config, _api_key, _sample_rate, _samples| Ok("current final".to_string()),
        )?;

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
        assert_eq!(
            receive_runtime_test_packet(&osc_receiver)?,
            typing_packet(false)
        );

        osc_receiver
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        assert!(matches!(
            receive_runtime_test_packet(&osc_receiver),
            Err(AppError::OscSend { .. })
        ));
        assert_eq!(
            receive_runtime_test_packet(&current_osc_receiver)?,
            chatbox_text_packet("current final")
        );
        assert_eq!(
            receive_runtime_test_packet(&current_osc_receiver)?,
            typing_packet(false)
        );

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
    fn capture_error_waits_for_in_flight_final_and_discards_queued_speech() -> AppResult<()> {
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

        let (osc_sender, osc_receiver) = runtime_test_sender_and_receiver()?;
        let activity = osc_sender.activity_handle();
        let generation = RuntimeGeneration::active();
        let (segment_sender, segment_receiver) = sync_channel(STT_QUEUE_CAPACITY);
        let (in_flight_sender, in_flight_receiver) = std::sync::mpsc::channel();
        let config = AppConfig::default();
        let http_client = Client::builder()
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;

        activity.utterance_started("in-flight")?;
        activity.utterance_started("queued")?;
        assert_eq!(
            receive_runtime_test_packet(&osc_receiver)?,
            typing_packet(true)
        );

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
                Some(osc_sender),
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
        assert_eq!(
            receive_runtime_test_packet(&osc_receiver)?,
            typing_packet(false)
        );

        activity.finish_after_error()?;

        osc_receiver
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        assert!(matches!(
            receive_runtime_test_packet(&osc_receiver),
            Err(AppError::OscSend { .. })
        ));

        Ok(())
    }

    #[test]
    fn typing_off_is_sent_before_the_stt_worker_join_begins() -> AppResult<()> {
        let (sender, receiver) = runtime_test_sender_and_receiver()?;
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);
        activity.request_stop(&cancel)?;

        let (typing_result, worker_result) = finish_chatbox_before_join(Some(&activity), || {
            let packet = receive_runtime_test_packet(&receiver)?;
            let expected = typing_packet(false);

            if packet == expected {
                Ok(())
            } else {
                Err(AppError::runtime(
                    "STT worker join began before typing-off was observable.",
                ))
            }
        });

        typing_result?;
        worker_result?;

        Ok(())
    }

    #[test]
    fn every_no_final_resolution_turns_typing_off() -> AppResult<()> {
        let reasons = [
            UtteranceEndReason::NoSpeech,
            UtteranceEndReason::SttFailed,
            UtteranceEndReason::Discarded,
        ];

        for (index, reason) in reasons.into_iter().enumerate() {
            let (sender, receiver) = runtime_test_sender_and_receiver()?;
            let activity = sender.activity_handle();
            let utterance_id = format!("no-final-{index}");
            let resolution = NoFinalUtteranceResolution {
                utterance_id: utterance_id.clone(),
                reason,
            };
            let mut emitted = None;

            activity.utterance_started(&utterance_id)?;
            assert_eq!(receive_runtime_test_packet(&receiver)?, typing_packet(true));

            complete_no_final_utterance(
                Some(&activity),
                resolution,
                |emitted_utterance_id, emitted_reason| {
                    emitted = Some((emitted_utterance_id, emitted_reason));
                },
            )?;

            let (emitted_utterance_id, emitted_reason) = emitted
                .ok_or_else(|| AppError::runtime("No-final completion event was not emitted."))?;
            assert_eq!(emitted_utterance_id, utterance_id);
            assert!(same_utterance_end_reason(emitted_reason, reason));
            assert_eq!(
                receive_runtime_test_packet(&receiver)?,
                typing_packet(false)
            );
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

    fn runtime_test_sender_and_receiver() -> AppResult<(ChatboxOscSender, UdpSocket)> {
        let receiver = UdpSocket::bind("127.0.0.1:0")
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let port = receiver
            .local_addr()
            .map_err(|error| AppError::osc_bind(error.to_string()))?
            .port();
        let config = OscConfig {
            host: "127.0.0.1".to_string(),
            port,
            enabled: true,
        };
        let sender = ChatboxOscSender::new(&config, ChatboxPacer::default())?;

        Ok((sender, receiver))
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

    fn receive_runtime_test_packet(receiver: &UdpSocket) -> AppResult<OscPacket> {
        let mut buffer = [0_u8; 1024];
        let (size, _) = receiver
            .recv_from(&mut buffer)
            .map_err(|error| AppError::osc_send("test receiver", error.to_string()))?;
        let (_, packet) = decoder::decode_udp(&buffer[..size])
            .map_err(|error| AppError::osc_encode(error.to_string()))?;

        Ok(packet)
    }

    fn typing_packet(is_typing: bool) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: "/chatbox/typing".to_string(),
            args: vec![OscType::Bool(is_typing)],
        })
    }

    fn chatbox_text_packet(text: &str) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: "/chatbox/input".to_string(),
            args: vec![
                OscType::String(text.to_string()),
                OscType::Bool(true),
                OscType::Bool(false),
            ],
        })
    }
}
