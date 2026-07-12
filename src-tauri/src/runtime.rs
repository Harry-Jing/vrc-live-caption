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
//! buffered and queued speech is discarded instead of drained, and no Chatbox
//! text is sent after the stop request. A state-clearing typing-off packet is
//! sent before waiting for an STT request that is already in flight, so runtime
//! commands must run off the main thread (`#[tauri::command(async)]`) to keep the
//! window responsive during that wait.

use crate::audio::{open_input_capture, receive_audio};
use crate::config::{AppConfig, OscConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, TranscriptUpdate, UtteranceEndReason,
    emit_diagnostic, emit_status, emit_transcript_final, emit_utterance_ended,
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
use tauri::AppHandle;

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
}

struct RuntimeHandle {
    stop_requested: Arc<AtomicBool>,
    chatbox_activity: Option<ChatboxActivityHandle>,
    join_handle: JoinHandle<()>,
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

impl Default for RuntimeManager {
    fn default() -> Self {
        Self {
            handle: Mutex::new(None),
        }
    }
}

impl RuntimeManager {
    pub(crate) fn start(&self, app: AppHandle, config: AppConfig) -> AppResult<()> {
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
        let osc_sender = match initialize_runtime_chatbox(&config.osc) {
            RuntimeChatboxInit::Disabled => None,
            RuntimeChatboxInit::Ready(sender) => Some(sender),
            RuntimeChatboxInit::Unavailable(error) => {
                emit_chatbox_activity_failure(&app, &error, "Chatbox OSC output could not start");
                None
            }
        };
        let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

        let stop_requested = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop_requested);
        let join_handle = thread::Builder::new()
            .name("vrc-live-caption-runtime".to_string())
            .spawn(move || run_runtime_thread(app, config, openai_api_key, osc_sender, thread_stop))
            .map_err(|error| {
                AppError::runtime(format!("Failed to start runtime thread: {error}"))
            })?;

        *guard = Some(RuntimeHandle {
            stop_requested,
            chatbox_activity,
            join_handle,
        });

        Ok(())
    }

    pub(crate) fn stop(&self, app: &AppHandle) -> AppResult<()> {
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

        if let Some(activity) = &handle.chatbox_activity {
            if let Err(error) = activity.request_stop(&handle.stop_requested) {
                handle.stop_requested.store(true, Ordering::Relaxed);
                emit_chatbox_activity_failure(app, &error, "Typing indicator cleanup failed");
            }
        } else {
            handle.stop_requested.store(true, Ordering::Relaxed);
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

fn initialize_runtime_chatbox(config: &OscConfig) -> RuntimeChatboxInit {
    if !config.enabled {
        return RuntimeChatboxInit::Disabled;
    }

    match ChatboxOscSender::new(config) {
        Ok(sender) => RuntimeChatboxInit::Ready(sender),
        Err(error) => RuntimeChatboxInit::Unavailable(error),
    }
}

fn run_runtime_thread(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    osc_sender: Option<ChatboxOscSender>,
    stop_requested: Arc<AtomicBool>,
) {
    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

    if let Err(error) = run_runtime(
        app.clone(),
        config,
        openai_api_key,
        osc_sender,
        stop_requested,
    ) {
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
    stop_requested: Arc<AtomicBool>,
) -> AppResult<()> {
    emit_status(
        &app,
        RuntimeStatus::Starting,
        Some("Starting outgoing caption runtime".to_string()),
    );

    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

    match config.stt.provider {
        SttProvider::Mock => run_mock_runtime(app, stop_requested, chatbox_activity.as_ref()),
        SttProvider::OpenAi => {
            let api_key = openai_api_key.ok_or_else(|| {
                AppError::secret("OpenAI API key was not loaded before runtime startup.")
            })?;

            run_openai_runtime(app, config, api_key, osc_sender, stop_requested)
        }
    }
}

fn run_mock_runtime(
    app: AppHandle,
    stop_requested: Arc<AtomicBool>,
    chatbox_activity: Option<&ChatboxActivityHandle>,
) -> AppResult<()> {
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

    while !stop_requested.load(Ordering::Relaxed) {
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
    stop_requested: Arc<AtomicBool>,
) -> AppResult<()> {
    let capture = open_input_capture(&config.audio)?;
    let sample_rate = capture.sample_rate;
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
        Arc::clone(&stop_requested),
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

    while !stop_requested.load(Ordering::Relaxed) {
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

    // Stop path: release the microphone before waiting on the worker, and
    // discard buffered tail speech instead of sending it to STT after stop.
    drop(stream);
    drop(segment_sender);
    let (typing_result, worker_result) =
        finish_chatbox_before_join(chatbox_activity.as_ref(), || {
            stt_worker
                .join()
                .map_err(|_| AppError::runtime("STT worker thread panicked while stopping."))
        });

    if let Err(error) = typing_result {
        emit_chatbox_activity_failure(&app, &error, "Typing indicator cleanup failed");
    }

    worker_result?;
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

    Ok(())
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
        Err(TrySendError::Disconnected(_)) => Err(AppError::runtime(
            "STT worker stopped unexpectedly while the runtime was still capturing audio.",
        )),
    }
}

fn spawn_stt_worker(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    http_client: Client,
    osc_sender: Option<ChatboxOscSender>,
    segment_receiver: Receiver<SpeechSegment>,
    stop_requested: Arc<AtomicBool>,
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
                stop_requested,
            )
        })
        .map_err(|error| AppError::runtime(format!("Failed to start STT worker thread: {error}")))
}

fn run_stt_worker(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    http_client: Client,
    mut osc_sender: Option<ChatboxOscSender>,
    segment_receiver: Receiver<SpeechSegment>,
    stop_requested: Arc<AtomicBool>,
) {
    let mut discarded_segments: usize = 0;
    let chatbox_activity = osc_sender.as_ref().map(ChatboxOscSender::activity_handle);

    while let Ok(segment) = segment_receiver.recv() {
        if stop_requested.load(Ordering::Relaxed) {
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
            &stop_requested,
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

fn transcribe_and_emit_final(
    app: &AppHandle,
    config: &AppConfig,
    openai_api_key: &SecretString,
    http_client: &Client,
    segment: SpeechSegment,
    osc_sender: Option<&mut ChatboxOscSender>,
    stop_requested: &AtomicBool,
) -> AppResult<()> {
    let chatbox_activity = osc_sender.as_ref().map(|sender| sender.activity_handle());

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

    let text = match transcribe_openai_wav(
        http_client,
        &config.stt,
        openai_api_key,
        segment.sample_rate,
        &segment.samples,
    ) {
        Ok(text) => text,
        Err(error) => {
            // Resolve the utterance for the UI; the caller reports error details.
            end_utterance_without_final(
                app,
                chatbox_activity.as_ref(),
                segment.utterance_id,
                UtteranceEndReason::SttFailed,
            );

            return Err(error);
        }
    };

    if text.is_empty() {
        end_utterance_without_final(
            app,
            chatbox_activity.as_ref(),
            segment.utterance_id,
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

        return Ok(());
    }

    let utterance_id = segment.utterance_id.clone();
    emit_transcript_final(
        app,
        TranscriptUpdate {
            utterance_id: segment.utterance_id,
            text: text.clone(),
            language: config.stt.language.clone(),
            provider: config.stt.provider.as_str().to_string(),
            revision: 1,
        },
    );

    // This segment was transcribed while stop was requested: keep the App
    // preview, but never send Chatbox output after the user asked to stop.
    if stop_requested.load(Ordering::Relaxed) {
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
    let attempt = osc_sender.send_final_paced(&utterance_id, &text, stop_requested);

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

fn emit_chatbox_send_skipped_on_stop(app: &AppHandle) {
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

fn end_utterance_without_final(
    app: &AppHandle,
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

fn resolve_chatbox_activity(
    app: &AppHandle,
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

fn emit_chatbox_activity_failure(app: &AppHandle, error: &AppError, message: &'static str) {
    tracing::warn!(
        code = error.code(),
        error_message = %error,
        "Chatbox typing indicator update failed"
    );
    emit_diagnostic(app, DiagnosticUpdate::from_error(error, message));
}

fn finish_chatbox_stop(app: &AppHandle, activity: Option<&ChatboxActivityHandle>) {
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

    #[test]
    fn disabled_osc_does_not_create_runtime_chatbox_output() {
        let config = crate::config::OscConfig {
            host: "does-not-resolve.invalid".to_string(),
            port: 9000,
            enabled: false,
            min_interval_ms: 500,
        };

        assert!(matches!(
            initialize_runtime_chatbox(&config),
            RuntimeChatboxInit::Disabled
        ));
    }

    #[test]
    fn unavailable_osc_output_does_not_become_a_runtime_start_error() {
        let config = crate::config::OscConfig {
            host: "[::1]".to_string(),
            port: 9000,
            enabled: true,
            min_interval_ms: 500,
        };

        assert!(matches!(
            initialize_runtime_chatbox(&config),
            RuntimeChatboxInit::Unavailable(_)
        ));
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
            min_interval_ms: 500,
        };
        let sender = ChatboxOscSender::new(&config)?;

        Ok((sender, receiver))
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
}
