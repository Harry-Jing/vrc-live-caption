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
//! output is sent after the stop request. Stop still waits for an STT request
//! that is already in flight, so runtime commands must run off the main thread
//! (`#[tauri::command(async)]`) to keep the window responsive during that wait.

use crate::audio::{open_input_capture, receive_audio};
use crate::config::{AppConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, TranscriptUpdate, UtteranceEndReason,
    emit_diagnostic, emit_status, emit_transcript_final, emit_utterance_ended,
    emit_utterance_started, next_utterance_id,
};
use crate::osc::ChatboxOscSender;
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
    join_handle: JoinHandle<()>,
}

struct SpeechSegment {
    utterance_id: String,
    sample_rate: u32,
    samples: Vec<f32>,
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
        clear_finished_runtime(&mut guard)?;

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        let openai_api_key = if matches!(config.stt.provider, SttProvider::OpenAi) {
            match load_openai_api_key() {
                Ok(api_key) => Some(api_key),
                Err(error) => {
                    let _ = emit_diagnostic(
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

        let stop_requested = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop_requested);
        let join_handle = thread::Builder::new()
            .name("vrc-live-caption-runtime".to_string())
            .spawn(move || run_runtime_thread(app, config, openai_api_key, thread_stop))
            .map_err(|error| {
                AppError::runtime(format!("Failed to start runtime thread: {error}"))
            })?;

        *guard = Some(RuntimeHandle {
            stop_requested,
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
            )?;
            return Ok(());
        };

        handle.stop_requested.store(true, Ordering::Relaxed);
        emit_status(
            app,
            RuntimeStatus::Stopping,
            Some("Stopping runtime and discarding pending speech".to_string()),
        )?;

        if handle.join_handle.join().is_err() {
            let error = AppError::runtime("Runtime thread panicked while stopping.");
            let _ = emit_status(app, RuntimeStatus::Error, Some(error.to_string()));
            return Err(error);
        }

        emit_status(
            app,
            RuntimeStatus::Stopped,
            Some("Runtime stopped".to_string()),
        )?;
        emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Runtime,
                "runtime.stopped",
                "Runtime stopped",
                "Microphone capture has been released.",
            ),
        )
    }
}

fn clear_finished_runtime(handle: &mut Option<RuntimeHandle>) -> AppResult<()> {
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

    handle
        .join_handle
        .join()
        .map_err(|_| AppError::runtime("Runtime thread panicked after stopping."))
}

fn run_runtime_thread(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    stop_requested: Arc<AtomicBool>,
) {
    if let Err(error) = run_runtime(app.clone(), config, openai_api_key, stop_requested) {
        tracing::warn!(
            code = error.code(),
            error_message = %error,
            "runtime stopped with error"
        );

        let _ = emit_status(&app, RuntimeStatus::Error, Some(error.to_string()));
        let _ = emit_diagnostic(
            &app,
            DiagnosticUpdate::from_error(&error, "Runtime stopped with an error"),
        );
    }
}

fn run_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: Option<SecretString>,
    stop_requested: Arc<AtomicBool>,
) -> AppResult<()> {
    emit_status(
        &app,
        RuntimeStatus::Starting,
        Some("Starting outgoing caption runtime".to_string()),
    )?;

    match config.stt.provider {
        SttProvider::Mock => run_mock_runtime(app, stop_requested),
        SttProvider::OpenAi => {
            let api_key = openai_api_key.ok_or_else(|| {
                AppError::secret("OpenAI API key was not loaded before runtime startup.")
            })?;

            run_openai_runtime(app, config, api_key, stop_requested)
        }
    }
}

fn run_mock_runtime(app: AppHandle, stop_requested: Arc<AtomicBool>) -> AppResult<()> {
    emit_status(
        &app,
        RuntimeStatus::Running,
        Some("Mock runtime is running".to_string()),
    )?;
    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Runtime,
            "runtime.mock_started",
            "Mock runtime started",
            "Use Mock Transcript to test normalized runtime events.",
        ),
    )?;

    while !stop_requested.load(Ordering::Relaxed) {
        thread::sleep(RECEIVE_TIMEOUT);
    }

    Ok(())
}

fn run_openai_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    stop_requested: Arc<AtomicBool>,
) -> AppResult<()> {
    let capture = open_input_capture(&config.audio)?;
    let sample_rate = capture.sample_rate;
    let (segment_sender, segment_receiver) = sync_channel(STT_QUEUE_CAPACITY);
    // Created once per runtime and reused by the worker across segments: the
    // HTTP client keeps its connection pool and the OSC sender its socket.
    let http_client = build_stt_client()?;
    let osc_sender = ChatboxOscSender::new(&config.osc)?;
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
    )?;
    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Audio,
            "audio.capture_started",
            "Microphone capture started",
            format!("Capturing mono audio at {sample_rate} Hz."),
        ),
    )?;

    while !stop_requested.load(Ordering::Relaxed) {
        let Some(samples) = receive_audio(&capture.receiver, RECEIVE_TIMEOUT)? else {
            if let Some(samples) = segmenter.tick(Instant::now()) {
                queue_speech_segment(
                    &app,
                    segmenter.sample_rate(),
                    samples,
                    &mut utterance_id,
                    &segment_sender,
                )?;
            }
            continue;
        };

        let update = segmenter.push_samples(samples, Instant::now());

        if update.speech_started {
            let next_utterance = next_utterance_id("speech");
            utterance_id = Some(next_utterance.clone());
            emit_utterance_started(&app, next_utterance)?;
        }

        if let Some(samples) = update.ready_segment {
            queue_speech_segment(
                &app,
                segmenter.sample_rate(),
                samples,
                &mut utterance_id,
                &segment_sender,
            )?;
        }
    }

    // Stop path: release the microphone before waiting on the worker, and
    // discard buffered tail speech instead of sending it to STT after stop.
    drop(stream);
    let tail_speech_discarded = segmenter.finish().is_some();

    drop(segment_sender);
    stt_worker
        .join()
        .map_err(|_| AppError::runtime("STT worker thread panicked while stopping."))?;

    if tail_speech_discarded {
        if let Some(utterance_id) = utterance_id {
            emit_utterance_ended(&app, utterance_id, UtteranceEndReason::Discarded)?;
        }

        emit_diagnostic(
            &app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.tail_speech_discarded",
                "Unsent speech discarded",
                "Speech captured just before stop was discarded without transcription.",
            ),
        )?;
    }

    Ok(())
}

fn queue_speech_segment(
    app: &AppHandle,
    sample_rate: u32,
    samples: Vec<f32>,
    utterance_id: &mut Option<String>,
    segment_sender: &SyncSender<SpeechSegment>,
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
            )?;

            emit_utterance_ended(app, segment.utterance_id, UtteranceEndReason::Discarded)
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
    osc_sender: ChatboxOscSender,
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
    mut osc_sender: ChatboxOscSender,
    segment_receiver: Receiver<SpeechSegment>,
    stop_requested: Arc<AtomicBool>,
) {
    let mut discarded_segments: usize = 0;

    while let Ok(segment) = segment_receiver.recv() {
        if stop_requested.load(Ordering::Relaxed) {
            discarded_segments += 1;
            let _ = emit_utterance_ended(&app, segment.utterance_id, UtteranceEndReason::Discarded);
            continue;
        }

        if let Err(error) = transcribe_and_emit_final(
            &app,
            &config,
            &openai_api_key,
            &http_client,
            segment,
            &mut osc_sender,
            &stop_requested,
        ) {
            tracing::warn!(
                code = error.code(),
                error_message = %error,
                "speech segment failed"
            );

            let _ = emit_diagnostic(
                &app,
                DiagnosticUpdate::from_error(&error, "Speech segment failed"),
            );
        }
    }

    if discarded_segments > 0 {
        tracing::info!(discarded_segments, "discarded queued speech on stop");

        let _ = emit_diagnostic(
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
    osc_sender: &mut ChatboxOscSender,
    stop_requested: &AtomicBool,
) -> AppResult<()> {
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
    )?;

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
            let _ = emit_utterance_ended(app, segment.utterance_id, UtteranceEndReason::SttFailed);

            return Err(error);
        }
    };

    if text.is_empty() {
        emit_utterance_ended(app, segment.utterance_id, UtteranceEndReason::NoSpeech)?;

        return emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.no_speech",
                "STT returned no speech",
                "The captured segment did not contain recognized words.",
            ),
        );
    }

    emit_transcript_final(
        app,
        TranscriptUpdate {
            utterance_id: segment.utterance_id,
            text: text.clone(),
            language: config.stt.language.clone(),
            provider: config.stt.provider.as_str().to_string(),
            revision: 1,
        },
    )?;

    // This segment was transcribed while stop was requested: keep the App
    // preview, but never send Chatbox output after the user asked to stop.
    if stop_requested.load(Ordering::Relaxed) {
        return emit_chatbox_send_skipped_on_stop(app);
    }

    if !config.osc.enabled {
        return emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Osc,
                "osc.output_disabled",
                "Chatbox output skipped",
                "OSC output is disabled in settings.",
            ),
        );
    }

    // The paced send also watches the stop flag itself: a stop requested
    // while the send is waiting out the pacing interval cancels the send
    // instead of flushing one more Chatbox message.
    match osc_sender.send_paced(&text, stop_requested) {
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
            )
        }
        Ok(None) => emit_chatbox_send_skipped_on_stop(app),
        Err(error) => {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Chatbox output failed"),
            )?;

            Err(error)
        }
    }
}

fn emit_chatbox_send_skipped_on_stop(app: &AppHandle) -> AppResult<()> {
    emit_diagnostic(
        app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.send_skipped_on_stop",
            "Chatbox send skipped",
            "Runtime stop was requested before this transcript could be sent.",
        ),
    )
}
