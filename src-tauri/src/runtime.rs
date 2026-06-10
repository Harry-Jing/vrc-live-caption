//! Runtime lifecycle for Phase 1 outgoing captions.
//!
//! The capture loop drains microphone samples and never performs blocking STT
//! upload work. Completed speech segments are sent to a bounded STT worker queue;
//! per-segment STT or OSC failures emit diagnostics and keep the runtime alive.
//! Startup failures such as invalid config or unavailable microphone remain fatal.
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
    DiagnosticCategory, DiagnosticSeverity, DiagnosticUpdate, RuntimeStatus, TranscriptUpdate,
    emit_diagnostic, emit_status, emit_transcript_final, emit_transcript_partial,
    next_utterance_id,
};
use crate::osc::send_paced_chatbox_osc;
use crate::secrets::openai_api_key as load_openai_api_key;
use crate::segmenter::SpeechSegmenter;
use crate::stt::transcribe_openai_wav;
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
const MIN_SEGMENT_SECONDS: f32 = 0.7;
const MAX_SEGMENT_SECONDS: f32 = 12.0;

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
                        DiagnosticUpdate {
                            category: DiagnosticCategory::Config,
                            severity: DiagnosticSeverity::Error,
                            code: "config.openai_api_key_missing",
                            message: "Cloud STT is not configured".to_string(),
                            detail: Some(error.to_string()),
                        },
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
            DiagnosticUpdate {
                category: DiagnosticCategory::Runtime,
                severity: DiagnosticSeverity::Info,
                code: "runtime.stopped",
                message: "Runtime stopped".to_string(),
                detail: Some("Microphone capture has been released.".to_string()),
            },
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
            DiagnosticUpdate {
                category: diagnostic_category_for_error(error.code()),
                severity: DiagnosticSeverity::Error,
                code: error.code(),
                message: "Runtime stopped with an error".to_string(),
                detail: Some(error.to_string()),
            },
        );
    }
}

fn diagnostic_category_for_error(code: &str) -> DiagnosticCategory {
    if code.starts_with("audio_") {
        DiagnosticCategory::Audio
    } else if code.starts_with("config_") || code.starts_with("secret_") {
        DiagnosticCategory::Config
    } else if code.starts_with("osc_") {
        DiagnosticCategory::Osc
    } else if code.starts_with("stt_") || code.starts_with("wav_") {
        DiagnosticCategory::Stt
    } else {
        DiagnosticCategory::Runtime
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
        DiagnosticUpdate {
            category: DiagnosticCategory::Runtime,
            severity: DiagnosticSeverity::Info,
            code: "runtime.mock_started",
            message: "Mock runtime started".to_string(),
            detail: Some("Use Mock Transcript to test normalized runtime events.".to_string()),
        },
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
    let stt_worker = spawn_stt_worker(
        app.clone(),
        config.clone(),
        openai_api_key,
        segment_receiver,
        Arc::clone(&stop_requested),
    )?;
    let mut segmenter = SpeechSegmenter::new(
        sample_rate,
        SPEECH_RMS_THRESHOLD,
        SILENCE_TIMEOUT,
        MIN_SEGMENT_SECONDS,
        MAX_SEGMENT_SECONDS,
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
        DiagnosticUpdate {
            category: DiagnosticCategory::Audio,
            severity: DiagnosticSeverity::Info,
            code: "audio.capture_started",
            message: "Microphone capture started".to_string(),
            detail: Some(format!("Capturing mono audio at {sample_rate} Hz.")),
        },
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
            emit_transcript_partial(
                &app,
                TranscriptUpdate {
                    utterance_id: next_utterance,
                    text: "Listening...".to_string(),
                    language: config.stt.language.clone(),
                    provider: config.stt.provider.as_str().to_string(),
                    revision: 1,
                },
            )?;
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
        emit_diagnostic(
            &app,
            DiagnosticUpdate {
                category: DiagnosticCategory::Stt,
                severity: DiagnosticSeverity::Info,
                code: "stt.tail_speech_discarded",
                message: "Unsent speech discarded".to_string(),
                detail: Some(
                    "Speech captured just before stop was discarded without transcription."
                        .to_string(),
                ),
            },
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
    let utterance_id = utterance_id
        .take()
        .unwrap_or_else(|| next_utterance_id("speech"));

    match segment_sender.try_send(SpeechSegment {
        utterance_id,
        sample_rate,
        samples,
    }) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(segment)) => emit_diagnostic(
            app,
            DiagnosticUpdate {
                category: DiagnosticCategory::Stt,
                severity: DiagnosticSeverity::Warning,
                code: "stt.segment_dropped",
                message: "Speech segment dropped".to_string(),
                detail: Some(format!(
                    "STT is still processing earlier audio, so {:.1} seconds of captured speech was skipped.",
                    segment.samples.len() as f32 / segment.sample_rate as f32
                )),
            },
        ),
        Err(TrySendError::Disconnected(_)) => Err(AppError::runtime(
            "STT worker stopped unexpectedly while the runtime was still capturing audio.",
        )),
    }
}

fn spawn_stt_worker(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
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
    segment_receiver: Receiver<SpeechSegment>,
    stop_requested: Arc<AtomicBool>,
) {
    let mut last_osc_send: Option<Instant> = None;
    let mut discarded_segments: usize = 0;

    while let Ok(segment) = segment_receiver.recv() {
        if stop_requested.load(Ordering::Relaxed) {
            discarded_segments += 1;
            continue;
        }

        if let Err(error) = transcribe_and_emit_final(
            &app,
            &config,
            &openai_api_key,
            segment,
            &mut last_osc_send,
            &stop_requested,
        ) {
            tracing::warn!(
                code = error.code(),
                error_message = %error,
                "speech segment failed"
            );

            let _ = emit_diagnostic(
                &app,
                DiagnosticUpdate {
                    category: diagnostic_category_for_error(error.code()),
                    severity: DiagnosticSeverity::Error,
                    code: error.code(),
                    message: "Speech segment failed".to_string(),
                    detail: Some(error.to_string()),
                },
            );
        }
    }

    if discarded_segments > 0 {
        tracing::info!(discarded_segments, "discarded queued speech on stop");

        let _ = emit_diagnostic(
            &app,
            DiagnosticUpdate {
                category: DiagnosticCategory::Stt,
                severity: DiagnosticSeverity::Info,
                code: "stt.queued_speech_discarded",
                message: "Queued speech discarded".to_string(),
                detail: Some(format!(
                    "Discarded {discarded_segments} speech segment(s) that were still waiting for STT when the runtime stopped."
                )),
            },
        );
    }
}

fn transcribe_and_emit_final(
    app: &AppHandle,
    config: &AppConfig,
    openai_api_key: &SecretString,
    segment: SpeechSegment,
    last_osc_send: &mut Option<Instant>,
    stop_requested: &AtomicBool,
) -> AppResult<()> {
    emit_diagnostic(
        app,
        DiagnosticUpdate {
            category: DiagnosticCategory::Stt,
            severity: DiagnosticSeverity::Info,
            code: "stt.segment_started",
            message: "Sending speech segment to STT".to_string(),
            detail: Some(format!(
                "Captured {:.1} seconds for final transcription.",
                segment.samples.len() as f32 / segment.sample_rate as f32
            )),
        },
    )?;

    let text = transcribe_openai_wav(
        &config.stt,
        openai_api_key,
        segment.sample_rate,
        &segment.samples,
    )?;

    if text.is_empty() {
        return emit_diagnostic(
            app,
            DiagnosticUpdate {
                category: DiagnosticCategory::Stt,
                severity: DiagnosticSeverity::Info,
                code: "stt.no_speech",
                message: "STT returned no speech".to_string(),
                detail: Some("The captured segment did not contain recognized words.".to_string()),
            },
        );
    }

    emit_transcript_final(
        app,
        TranscriptUpdate {
            utterance_id: segment.utterance_id,
            text: text.clone(),
            language: config.stt.language.clone(),
            provider: config.stt.provider.as_str().to_string(),
            revision: 2,
        },
    )?;

    // This segment was transcribed while stop was requested: keep the App
    // preview, but never send Chatbox output after the user asked to stop.
    if stop_requested.load(Ordering::Relaxed) {
        return emit_diagnostic(
            app,
            DiagnosticUpdate {
                category: DiagnosticCategory::Osc,
                severity: DiagnosticSeverity::Info,
                code: "osc.send_skipped_on_stop",
                message: "Chatbox send skipped".to_string(),
                detail: Some(
                    "Runtime stop was requested before this transcript could be sent.".to_string(),
                ),
            },
        );
    }

    if !config.osc.enabled {
        return emit_diagnostic(
            app,
            DiagnosticUpdate {
                category: DiagnosticCategory::Osc,
                severity: DiagnosticSeverity::Info,
                code: "osc.output_disabled",
                message: "Chatbox output skipped".to_string(),
                detail: Some("OSC output is disabled in settings.".to_string()),
            },
        );
    }

    match send_paced_chatbox_osc(&config.osc, &text, last_osc_send) {
        Ok(result) => {
            let clipped_note = if result.clipped {
                " Text was clipped to fit the VRChat Chatbox layout."
            } else {
                ""
            };

            emit_diagnostic(
                app,
                DiagnosticUpdate {
                    category: DiagnosticCategory::Osc,
                    severity: DiagnosticSeverity::Info,
                    code: "osc.final_sent",
                    message: "Final transcript sent to Chatbox".to_string(),
                    detail: Some(format!(
                        "Sent {} bytes to {}.{}",
                        result.byte_count, result.target, clipped_note
                    )),
                },
            )
        }
        Err(error) => {
            emit_diagnostic(
                app,
                DiagnosticUpdate {
                    category: DiagnosticCategory::Osc,
                    severity: DiagnosticSeverity::Error,
                    code: error.code(),
                    message: "Chatbox output failed".to_string(),
                    detail: Some(error.to_string()),
                },
            )?;

            Err(error)
        }
    }
}
