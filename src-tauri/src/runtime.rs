//! Runtime lifecycle for outgoing captions.
//!
//! The runtime owns one microphone, one selected RecognitionSession, and one
//! publication policy per generation. Application VAD frames speech units while
//! audio and normalized recognition events flow through the same worker thread.
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

use crate::audio::{open_input_capture, receive_audio};
use crate::audio_level::AudioLevelMeter;
use crate::capability_planner::{ResolvedPublicationPolicy, RuntimePlanSnapshot, plan_runtime};
use crate::caption_session::{CaptionSessionSnapshotV1, CaptionSessionStore};
use crate::chatbox_diagnostics::{completed_publisher_diagnostic, live_publisher_diagnostic};
use crate::chatbox_pacer::ChatboxPacer;
use crate::chatbox_publication::RuntimeChatboxPublisher;
use crate::chatbox_publisher::{
    CompletedChatboxPublisher, CompletedPublisherEvent, PublisherReporter,
};
use crate::chatbox_publisher_common::{PublisherCloseReason, PublisherSubmitOutcome};
use crate::chatbox_transport::ChatboxTransport;
use crate::config::{AppConfig, OscConfig};
use crate::error::{AppError, AppResult};
use crate::events::{
    AudioLevelEvent, DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, UtteranceEndReason,
    emit_audio_level, emit_diagnostic, emit_status, emit_utterance_ended, emit_utterance_started,
    next_utterance_id, now_ms,
};
use crate::host_resolver::HostResolver;
use crate::live_chatbox_publisher::{LiveChatboxPublisher, LivePublisherReporter};
use crate::openai_realtime::OpenAiRealtimeSessionContext;
use crate::openai_realtime_transport::connect_openai_realtime_session;
use crate::osc::ChatboxOscSender;
use crate::recognition::{
    RecognitionAudioChunk, RecognitionEndReason, RecognitionEvent, RecognitionSession,
};
use crate::reconnect::{ReconnectDecision, ReconnectSupervisor};
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeCredentialSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase,
    RuntimeSessionSnapshot,
};
use crate::runtime_generation::{ChatboxPublisherBoundary, RuntimeGeneration};
use crate::segmenter::SpeechSegmenter;
use secrecy::SecretString;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Runtime};

const RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
const RECOGNITION_COMMAND_QUEUE_CAPACITY: usize = 32;
const RECOGNITION_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
pub(crate) const SPEECH_RMS_THRESHOLD: f32 = 0.012;
const SILENCE_TIMEOUT: Duration = Duration::from_millis(1200);
// Voiced audio only; long enough to drop clicks and pops, short enough to
// keep one-word utterances such as "Yes".
const MIN_VOICED_SECONDS: f32 = 0.3;
// This is only the absolute fallback for uninterrupted speech; the 1.2-second
// silence boundary still closes normal utterances earlier. Prior VRChat
// testing found that 12 seconds split an approximately 20-second thought even
// though both ordered units were preserved, so cloud recognition uses 30
// seconds. Keep this internal and re-measure latency before raising it again.
const MAX_SEGMENT_SECONDS: f32 = 30.0;
const PREROLL_SECONDS: f32 = 0.25;

fn new_recognition_segmenter(sample_rate: u32) -> SpeechSegmenter {
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
    pub(crate) runtime_plan: RuntimePlanSnapshot,
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

enum RuntimePublisherInit {
    Disabled,
    Ready(RuntimeChatboxPublisher),
    Unavailable(AppError),
}

enum RecognitionCommand {
    StartUnit {
        unit_id: String,
        started_at_ms: u64,
        sample_rate_hz: u32,
        initial_audio: Vec<f32>,
    },
    AppendAudio {
        sample_rate_hz: u32,
        samples: Vec<f32>,
    },
    EndInput,
}

#[derive(Clone, Default)]
struct ConnectionAttemptCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl ConnectionAttemptCancelToken {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

fn publisher_boundary(
    publisher: Option<&RuntimeChatboxPublisher>,
) -> Option<&dyn ChatboxPublisherBoundary> {
    publisher.map(|publisher| publisher as &dyn ChatboxPublisherBoundary)
}

fn publisher_failure_message<'a>(
    publisher: Option<&RuntimeChatboxPublisher>,
    completed: &'a str,
    live: &'a str,
) -> &'a str {
    match publisher {
        Some(RuntimeChatboxPublisher::Live(_)) => live,
        Some(RuntimeChatboxPublisher::Completed(_)) | None => completed,
    }
}

fn resolve_runtime_publication_policy(
    config: &AppConfig,
    runtime_plan: &RuntimePlanSnapshot,
) -> AppResult<ResolvedPublicationPolicy> {
    if runtime_plan != &plan_runtime(config) {
        return Err(AppError::config(
            "Runtime plan did not match the selected backend configuration.",
        ));
    }

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

impl RuntimeGeneration {
    pub(crate) fn submit_recognition_event<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        event: RecognitionEvent,
    ) -> AppResult<bool> {
        let mut submit_result = None;
        let committed = self.commit_if_active(|| {
            submit_result = Some(self.submit_recognition_event_at_boundary(app, publisher, event));
        })?;
        if !committed {
            return Ok(false);
        }

        submit_result.ok_or_else(|| {
            AppError::state("Runtime recognition event commit did not produce a result.")
        })?
    }

    fn submit_recognition_event_at_boundary<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        event: RecognitionEvent,
    ) -> AppResult<bool> {
        match event {
            RecognitionEvent::UnitStarted {
                generation,
                stream_id,
                unit_id,
                started_at_ms,
            } => {
                let Some(snapshot) = self.start_caption_unit(
                    generation,
                    &stream_id,
                    unit_id.clone(),
                    started_at_ms,
                )?
                else {
                    return Ok(false);
                };
                self.report_accepted_snapshot(app, publisher, snapshot);
                emit_utterance_started(app, generation, stream_id, unit_id.clone(), started_at_ms);

                if let Some(publisher) = publisher
                    && let Err(error) = publisher
                        .try_submit_completed_event(CompletedPublisherEvent::Started { unit_id })
                {
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::from_error(&error, "Chatbox activity could not start"),
                    );
                }
            }
            RecognitionEvent::UnitEnded {
                generation,
                stream_id,
                unit_id,
                reason,
            } => {
                let (reason, failure_detail) = match reason {
                    RecognitionEndReason::NoSpeech => (UtteranceEndReason::NoSpeech, None),
                    RecognitionEndReason::Failed { detail } => {
                        (UtteranceEndReason::SttFailed, Some(detail))
                    }
                };
                let Some(snapshot) =
                    self.end_caption_unit_without_caption(generation, &stream_id, &unit_id)?
                else {
                    return Ok(false);
                };
                self.report_accepted_snapshot(app, publisher, snapshot);
                emit_utterance_ended(
                    app,
                    generation,
                    stream_id,
                    unit_id.clone(),
                    reason,
                    now_ms(),
                );
                if let Some(detail) = failure_detail {
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::error(
                            DiagnosticCategory::Stt,
                            "stt.item_failed",
                            "One utterance could not be transcribed",
                            detail,
                        ),
                    );
                }
                if let Some(publisher) = publisher
                    && let Err(error) = publisher
                        .try_submit_completed_event(CompletedPublisherEvent::Aborted { unit_id })
                {
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::from_error(&error, "Chatbox activity could not resolve"),
                    );
                }
            }
            RecognitionEvent::Caption(caption) => {
                let unit_id = caption.unit_id.clone();
                let text = caption.text.clone();
                let is_completed = caption.state == crate::caption_session::CaptionState::Completed;
                let Some(snapshot) = self.accept_caption(caption)? else {
                    return Ok(false);
                };
                self.report_accepted_snapshot(app, publisher, snapshot);

                if is_completed && let (Some(publisher), Some(unit_id)) = (publisher, unit_id) {
                    submit_completed_chatbox_candidate(app, publisher, self, unit_id, text);
                }
            }
        }

        Ok(true)
    }

    fn abort_active_units_for_reconnect<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
    ) -> AppResult<()> {
        let snapshot = self.caption_snapshot()?;
        for active_unit in snapshot.active_units {
            let _accepted = self.submit_recognition_event(
                app,
                publisher,
                RecognitionEvent::UnitEnded {
                    generation: self.generation_id(),
                    stream_id: self.stream_id().to_string(),
                    unit_id: active_unit.unit_id,
                    reason: RecognitionEndReason::Failed {
                        detail: "Speech was discarded because the recognition connection was interrupted."
                            .to_string(),
                    },
                },
            )?;
        }
        Ok(())
    }

    fn report_accepted_snapshot<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        snapshot: CaptionSessionSnapshotV1,
    ) {
        // The App and Live publication observe the exact same store-accepted
        // aggregate. Completed publication deliberately ignores this input and
        // keeps its existing lossless lifecycle event path.
        self.report_caption_snapshot(snapshot.clone());
        let Some(publisher) = publisher else {
            return;
        };

        match publisher.observe_snapshot(&snapshot) {
            Ok(PublisherSubmitOutcome::Handled) => {}
            Ok(PublisherSubmitOutcome::Closed) => {
                if !self.is_hard_stop_requested() {
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::info(
                            DiagnosticCategory::Osc,
                            "osc.live_snapshot_discarded_after_close",
                            "Live Chatbox snapshot discarded",
                            "The Live publisher closed before this accepted App caption snapshot could be observed.",
                        ),
                    );
                }
            }
            Err(error) => emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Live Chatbox snapshot could not be observed"),
            ),
        }
    }
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
            runtime_plan,
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
        let publication_policy = resolve_runtime_publication_policy(&config, &runtime_plan)?;

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
            emit_status(
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

fn initialize_runtime_publisher<R: Runtime>(
    app: &AppHandle<R>,
    config: &OscConfig,
    policy: ResolvedPublicationPolicy,
    chatbox_pacer: ChatboxPacer,
    generation: RuntimeGeneration,
    host_resolver: &HostResolver,
    is_cancelled: &dyn Fn() -> bool,
) -> RuntimePublisherInit {
    if !config.enabled {
        return RuntimePublisherInit::Disabled;
    }

    let sender = match ChatboxOscSender::new(config, host_resolver, is_cancelled) {
        Ok(sender) => sender,
        Err(error) => return RuntimePublisherInit::Unavailable(error),
    };
    let transport: Arc<dyn ChatboxTransport> = Arc::new(sender);
    let publisher = match policy {
        ResolvedPublicationPolicy::Completed => {
            let reporter_app = app.clone();
            let reporter: PublisherReporter = Arc::new(move |diagnostic| {
                emit_diagnostic(&reporter_app, completed_publisher_diagnostic(diagnostic));
            });
            CompletedChatboxPublisher::start(transport, chatbox_pacer, generation, reporter)
                .map(RuntimeChatboxPublisher::Completed)
        }
        ResolvedPublicationPolicy::LiveUnit { .. } => {
            let reporter_app = app.clone();
            let reporter: LivePublisherReporter = Arc::new(move |diagnostic| {
                emit_diagnostic(&reporter_app, live_publisher_diagnostic(diagnostic));
            });
            LiveChatboxPublisher::start(transport, chatbox_pacer, generation, policy, reporter)
                .map(RuntimeChatboxPublisher::Live)
        }
    };

    match publisher {
        Ok(publisher) => RuntimePublisherInit::Ready(publisher),
        Err(error) => RuntimePublisherInit::Unavailable(error),
    }
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
    publisher: Option<&RuntimeChatboxPublisher>,
    reason: PublisherCloseReason,
) {
    let close_result = match reason {
        PublisherCloseReason::Stop => generation.request_stop(publisher_boundary(publisher)),
        PublisherCloseReason::RuntimeError => match publisher {
            Some(publisher) => generation
                .close_publisher_at_boundary(Some(publisher), PublisherCloseReason::RuntimeError),
            None => Ok(()),
        },
    };
    if let Err(error) = close_result {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(
                &error,
                publisher_failure_message(
                    publisher,
                    "Completed publisher could not close",
                    "Live publisher could not close",
                ),
            ),
        );
    }

    if let Some(publisher) = publisher
        && let Err(error) = publisher.join()
    {
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

    if let Err(error) = generation.close_caption_session_at_boundary() {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Caption session could not close"),
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
        emit_status(
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

    let mut reconnect = ReconnectSupervisor::default();
    let mut audio_level_revision = 0_u64;
    loop {
        if generation.is_work_cancelled() || generation.is_hard_stop_requested() {
            return Ok(());
        }

        let connection_epoch = reconnect.begin_connection_attempt();
        let context = OpenAiRealtimeSessionContext {
            generation: generation.generation_id(),
            connection_epoch,
            stream_id: generation.stream_id().to_string(),
        };
        let connection_result = connect_openai_realtime_session(
            context,
            config.stt.model,
            config.stt.languages.clone(),
            &openai_api_key,
            &host_resolver,
            &|| generation.is_work_cancelled(),
        );
        let (attempt_result, connected_for) = match connection_result {
            Ok(recognition) => {
                let connected_at = Instant::now();
                let result = run_connected_openai_attempt(
                    &app,
                    &config,
                    publisher.as_ref(),
                    &generation,
                    recognition,
                    &mut reconnect,
                    &mut audio_level_revision,
                );
                (result, Some(connected_at.elapsed()))
            }
            Err(error) => (Err(error), None),
        };

        if generation.is_hard_stop_requested() {
            return Ok(());
        }

        let error = match attempt_result {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let ReconnectDecision::Retry { attempt, delay } =
            reconnect.on_failure(&error, connected_for, reconnect_jitter_percent())
        else {
            return Err(error);
        };

        generation.abort_active_units_for_reconnect(&app, publisher.as_ref())?;
        let delay_ms = delay.as_millis();
        let reconnecting = generation.commit_if_active(|| {
            emit_status(
                &app,
                RuntimeStatus::Reconnecting,
                Some(format!(
                    "Recognition connection interrupted; retry {attempt} in {delay_ms} ms"
                )),
            );
            emit_diagnostic(
                &app,
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
        if !reconnecting || !wait_for_reconnect(&generation, delay) {
            return Ok(());
        }
    }
}

fn run_connected_openai_attempt<R: Runtime, S: RecognitionSession + 'static>(
    app: &AppHandle<R>,
    config: &AppConfig,
    publisher: Option<&RuntimeChatboxPublisher>,
    generation: &RuntimeGeneration,
    mut recognition: S,
    reconnect: &mut ReconnectSupervisor,
    audio_level_revision: &mut u64,
) -> AppResult<()> {
    if generation.is_hard_stop_requested() {
        recognition.stop()?;
        return Ok(());
    }

    let capture = match open_input_capture(&config.audio) {
        Ok(capture) => capture,
        Err(error) => {
            if let Err(stop_error) = recognition.stop() {
                tracing::warn!(
                    code = stop_error.code(),
                    error_message = %stop_error,
                    "Recognition session also failed while closing after microphone startup failed"
                );
            }
            return Err(error);
        }
    };
    if generation.is_hard_stop_requested() {
        recognition.stop()?;
        return Ok(());
    }

    let sample_rate = capture.sample_rate;
    let mut audio_level = AudioLevelMeter::new(sample_rate, SPEECH_RMS_THRESHOLD)
        .map_err(|error| AppError::audio(error.to_string()))?;
    let reconnected = reconnect.is_recovery();
    let running = generation.commit_if_active(|| {
        emit_status(
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
        if reconnected {
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
        recognition.stop()?;
        return Ok(());
    }
    reconnect.mark_running();

    let attempt = ConnectionAttemptCancelToken::default();
    let (recognition_sender, recognition_receiver) =
        sync_channel(RECOGNITION_COMMAND_QUEUE_CAPACITY);
    let recognition_worker = spawn_recognition_worker(
        app.clone(),
        publisher.cloned(),
        generation.clone(),
        attempt.clone(),
        recognition,
        recognition_receiver,
    )?;
    let mut segmenter = new_recognition_segmenter(sample_rate);
    let stream = capture.stream;

    let runtime_result = (|| -> AppResult<()> {
        while !generation.is_work_cancelled() && !attempt.is_cancelled() {
            match receive_audio(&capture.receiver, RECEIVE_TIMEOUT)? {
                Some(samples) => {
                    for reading in audio_level.push_samples(&samples) {
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
                            attempt.cancel();
                            break;
                        }
                        *audio_level_revision = next_revision;
                    }
                    if attempt.is_cancelled() {
                        continue;
                    }
                    apply_segmenter_updates(
                        &attempt,
                        &recognition_sender,
                        segmenter.sample_rate(),
                        segmenter.push_samples(samples, Instant::now()),
                    )?;
                }
                None => apply_segmenter_updates(
                    &attempt,
                    &recognition_sender,
                    segmenter.sample_rate(),
                    [segmenter.tick(Instant::now())],
                )?,
            }
        }

        Ok(())
    })();

    attempt.cancel();
    drop(stream);
    let tail_speech_discarded = segmenter.discard_open_tail().speech_ended;
    drop(recognition_sender);
    let worker_result = recognition_worker
        .join()
        .map_err(|_| AppError::runtime("Recognition worker thread panicked while stopping."))?;
    if tail_speech_discarded && !generation.is_hard_stop_requested() {
        emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.tail_speech_discarded",
                "Uncommitted speech discarded",
                "Speech still open when recognition stopped was discarded without a completed caption.",
            ),
        );
    }

    match (runtime_result, worker_result) {
        (Err(runtime_error), Err(worker_error)) => {
            tracing::warn!(
                code = worker_error.code(),
                error_message = %worker_error,
                "Recognition worker also failed while closing after a runtime error"
            );
            Err(runtime_error)
        }
        (Err(runtime_error), Ok(())) => Err(runtime_error),
        (Ok(()), Err(worker_error)) if !generation.is_hard_stop_requested() => Err(worker_error),
        (Ok(()), Err(_)) | (Ok(()), Ok(())) => Ok(()),
    }
}

fn reconnect_jitter_percent() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .subsec_nanos();
    80 + nanos % 41
}

fn wait_for_reconnect(generation: &RuntimeGeneration, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    while !generation.is_work_cancelled() && !generation.is_hard_stop_requested() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        thread::sleep(remaining.min(RECEIVE_TIMEOUT));
    }
    false
}

fn apply_segmenter_updates(
    attempt: &ConnectionAttemptCancelToken,
    recognition_sender: &SyncSender<RecognitionCommand>,
    sample_rate_hz: u32,
    updates: impl IntoIterator<Item = crate::segmenter::SegmenterUpdate>,
) -> AppResult<()> {
    for update in updates {
        apply_segmenter_update(attempt, recognition_sender, sample_rate_hz, update)?;
    }
    Ok(())
}

fn apply_segmenter_update(
    attempt: &ConnectionAttemptCancelToken,
    recognition_sender: &SyncSender<RecognitionCommand>,
    sample_rate_hz: u32,
    update: crate::segmenter::SegmenterUpdate,
) -> AppResult<()> {
    if update.speech_started {
        return send_recognition_command(
            attempt,
            recognition_sender,
            RecognitionCommand::StartUnit {
                unit_id: next_utterance_id("speech"),
                started_at_ms: now_ms(),
                sample_rate_hz,
                initial_audio: update.audio,
            },
        )
        .and_then(|()| {
            if update.speech_ended {
                send_recognition_command(attempt, recognition_sender, RecognitionCommand::EndInput)
            } else {
                Ok(())
            }
        });
    }
    if !update.audio.is_empty() {
        send_recognition_command(
            attempt,
            recognition_sender,
            RecognitionCommand::AppendAudio {
                sample_rate_hz,
                samples: update.audio,
            },
        )?;
    }
    if update.speech_ended {
        send_recognition_command(attempt, recognition_sender, RecognitionCommand::EndInput)?;
    }
    Ok(())
}

fn send_recognition_command(
    attempt: &ConnectionAttemptCancelToken,
    sender: &SyncSender<RecognitionCommand>,
    command: RecognitionCommand,
) -> AppResult<()> {
    match sender.try_send(command) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            attempt.cancel();
            Err(AppError::stt_backpressure(
                "The recognition backend could not keep up with microphone audio; the bounded recognition queue filled, so the session stopped instead of silently dropping audio.",
            ))
        }
        Err(TrySendError::Disconnected(_)) => {
            attempt.cancel();
            Ok(())
        }
    }
}

fn spawn_recognition_worker<R: Runtime, S: RecognitionSession + 'static>(
    app: AppHandle<R>,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    attempt: ConnectionAttemptCancelToken,
    recognition: S,
    receiver: Receiver<RecognitionCommand>,
) -> AppResult<JoinHandle<AppResult<()>>> {
    thread::Builder::new()
        .name("vrc-live-caption-recognition".to_string())
        .spawn(move || {
            run_recognition_worker(app, publisher, generation, attempt, recognition, receiver)
        })
        .map_err(|error| {
            AppError::runtime(format!(
                "Failed to start recognition worker thread: {error}"
            ))
        })
}

fn run_recognition_worker<R: Runtime>(
    app: AppHandle<R>,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    attempt: ConnectionAttemptCancelToken,
    mut recognition: impl RecognitionSession,
    receiver: Receiver<RecognitionCommand>,
) -> AppResult<()> {
    let work_result = (|| -> AppResult<()> {
        while !generation.is_work_cancelled() && !attempt.is_cancelled() {
            match receiver.recv_timeout(RECOGNITION_EVENT_POLL_INTERVAL) {
                Ok(RecognitionCommand::StartUnit {
                    unit_id,
                    started_at_ms,
                    sample_rate_hz,
                    initial_audio,
                }) => {
                    let event = recognition.start_unit(unit_id, started_at_ms)?;
                    if !generation.submit_recognition_event(&app, publisher.as_ref(), event)? {
                        return Ok(());
                    }
                    recognition.append_audio(RecognitionAudioChunk {
                        sample_rate_hz,
                        samples: &initial_audio,
                    })?;
                }
                Ok(RecognitionCommand::AppendAudio {
                    sample_rate_hz,
                    samples,
                }) => recognition.append_audio(RecognitionAudioChunk {
                    sample_rate_hz,
                    samples: &samples,
                })?,
                Ok(RecognitionCommand::EndInput) => recognition.end_input()?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }

            for event in recognition.drain_events(now_ms())? {
                if !generation.submit_recognition_event(&app, publisher.as_ref(), event)? {
                    return Ok(());
                }
            }
        }
        Ok(())
    })();

    attempt.cancel();
    let stop_result = recognition.stop();
    match (work_result, stop_result) {
        (Err(work_error), Err(stop_error)) => {
            tracing::warn!(
                code = stop_error.code(),
                error_message = %stop_error,
                "Recognition session also failed while closing after a worker error"
            );
            Err(work_error)
        }
        (Err(work_error), Ok(())) => Err(work_error),
        (Ok(()), Err(stop_error)) if !generation.is_hard_stop_requested() => Err(stop_error),
        (Ok(()), Err(_)) | (Ok(()), Ok(())) => Ok(()),
    }
}

fn emit_chatbox_send_skipped_on_stop<R: Runtime>(app: &AppHandle<R>) {
    emit_diagnostic(
        app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.send_skipped_on_stop",
            "Chatbox send skipped",
            "Runtime stop was requested before this caption could be sent.",
        ),
    );
}

fn submit_completed_chatbox_candidate<R: Runtime>(
    app: &AppHandle<R>,
    publisher: &RuntimeChatboxPublisher,
    generation: &RuntimeGeneration,
    unit_id: String,
    text: String,
) {
    match publisher.try_submit_completed_event(CompletedPublisherEvent::Completed { unit_id, text })
    {
        Ok(PublisherSubmitOutcome::Handled) => {}
        Ok(PublisherSubmitOutcome::Closed) => {
            if generation.is_hard_stop_requested() {
                emit_chatbox_send_skipped_on_stop(app);
            } else {
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::info(
                        DiagnosticCategory::Osc,
                        "osc.completed_unit_discarded_after_close",
                        "Completed Chatbox publication discarded",
                        "The runtime output worker closed before this completed caption could enter its queue. The App caption remains available.",
                    ),
                );
            }
        }
        Err(error) => emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Completed Chatbox publication was rejected"),
        ),
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
