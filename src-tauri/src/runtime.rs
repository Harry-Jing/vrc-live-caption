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
use crate::capability_planner::{ResolvedPublicationPolicy, RuntimePlanSnapshot, plan_runtime};
use crate::caption_session::{CaptionSessionSnapshotV1, CaptionSessionStore};
use crate::chatbox_pacer::ChatboxPacer;
use crate::chatbox_publication::{ChatboxPublisherBoundary, RuntimeChatboxPublisher};
use crate::chatbox_publisher::{
    ChatboxTransport, CompletedChatboxPublisher, CompletedPublisherEvent, PublisherCloseReason,
    PublisherDiagnostic, PublisherReporter, PublisherSubmitOutcome,
};
use crate::config::{AppConfig, OscConfig};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, UtteranceEndReason,
    emit_caption_session_changed, emit_diagnostic, emit_status, emit_utterance_ended,
    emit_utterance_started, next_utterance_id, now_ms,
};
use crate::live_chatbox_publisher::{
    LiveChatboxPublisher, LivePublisherDiagnostic, LivePublisherReporter,
};
use crate::openai_realtime::OpenAiRealtimeSessionContext;
use crate::openai_realtime_transport::connect_openai_realtime_session;
use crate::osc::ChatboxOscSender;
use crate::recognition::{
    RecognitionAudioChunk, RecognitionEndReason, RecognitionEvent, RecognitionSession,
};
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeCredentialSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase,
    RuntimeSessionSnapshot,
};
use crate::segmenter::SpeechSegmenter;
use secrecy::SecretString;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};

const RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
const RECOGNITION_COMMAND_QUEUE_CAPACITY: usize = 32;
const RECOGNITION_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SPEECH_RMS_THRESHOLD: f32 = 0.012;
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
}

pub(crate) struct RuntimeStartRequest {
    pub(crate) config: AppConfig,
    pub(crate) runtime_plan: RuntimePlanSnapshot,
    pub(crate) chatbox_pacer: ChatboxPacer,
    pub(crate) caption_session: CaptionSessionStore,
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

#[derive(Clone)]
pub(crate) struct RuntimeGeneration {
    generation_id: u64,
    stream_id: String,
    caption_session: CaptionSessionStore,
    caption_reporter: CaptionSessionReporter,
    output_gate: Arc<Mutex<()>>,
    hard_stop_requested: Arc<AtomicBool>,
    work_cancelled: Arc<AtomicBool>,
}

enum RuntimePublisherInit {
    Disabled,
    Ready(RuntimeChatboxPublisher),
    Unavailable(AppError),
}

enum RecognitionCommand {
    Start {
        unit_id: String,
        started_at_ms: u64,
        sample_rate_hz: u32,
        initial_audio: Vec<f32>,
    },
    Audio {
        sample_rate_hz: u32,
        samples: Vec<f32>,
    },
    EndInput,
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

type CaptionSessionReporter = Arc<dyn Fn(CaptionSessionSnapshotV1) + Send + Sync>;

impl RuntimeGeneration {
    fn activate<R: Runtime>(
        app: &AppHandle<R>,
        generation_id: u64,
        caption_session: CaptionSessionStore,
    ) -> AppResult<Self> {
        let snapshot = caption_session.begin_generation(generation_id)?;
        let active = snapshot.active.as_ref().ok_or_else(|| {
            AppError::state("Caption session rejected a non-monotonic runtime generation.")
        })?;
        if active.generation != generation_id {
            return Err(AppError::state(
                "Caption session activated a different runtime generation.",
            ));
        }
        let stream_id = active.stream_id.clone();

        let reporter_app = app.clone();
        let caption_reporter: CaptionSessionReporter = Arc::new(move |snapshot| {
            emit_caption_session_changed(&reporter_app, snapshot);
        });
        caption_reporter(snapshot);

        Ok(Self {
            generation_id,
            stream_id,
            caption_session,
            caption_reporter,
            output_gate: Arc::new(Mutex::new(())),
            hard_stop_requested: Arc::new(AtomicBool::new(false)),
            work_cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    #[cfg(test)]
    pub(crate) fn active() -> Self {
        let caption_session = CaptionSessionStore::default();
        if let Err(error) = caption_session.begin_generation(1) {
            tracing::error!(error_message = %error, "test caption session could not start");
        }

        Self {
            generation_id: 1,
            stream_id: "recognition-1-1".to_string(),
            caption_session,
            caption_reporter: Arc::new(|_| {}),
            output_gate: Arc::new(Mutex::new(())),
            hard_stop_requested: Arc::new(AtomicBool::new(false)),
            work_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn request_stop(
        &self,
        publisher: Option<&dyn ChatboxPublisherBoundary>,
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
        let publisher_result =
            self.close_publisher_at_boundary(publisher, PublisherCloseReason::Stop);
        let caption_result = self.close_caption_session_at_boundary();

        match (publisher_result, caption_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(publisher_error), Err(caption_error)) => Err(AppError::state(format!(
                "Runtime outputs could not close cleanly: {publisher_error} {caption_error}"
            ))),
        }
    }

    fn close_publisher_at_boundary(
        &self,
        publisher: Option<&dyn ChatboxPublisherBoundary>,
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

    pub(crate) fn submit_recognition_event<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        event: RecognitionEvent,
    ) -> AppResult<bool> {
        let _output_gate = self
            .output_gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation lock was poisoned."))?;
        if self.hard_stop_requested.load(Ordering::SeqCst) {
            return Ok(false);
        }

        match event {
            RecognitionEvent::UnitStarted {
                generation,
                stream_id,
                unit_id,
                started_at_ms,
            } => {
                let Some(snapshot) = self.caption_session.start_unit(
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
                let Some(snapshot) = self
                    .caption_session
                    .end_unit_without_caption(generation, &stream_id, &unit_id)?
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
                let Some(snapshot) = self.caption_session.accept_caption(caption)? else {
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

    fn report_accepted_snapshot<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        snapshot: CaptionSessionSnapshotV1,
    ) {
        // The App and Live publication observe the exact same store-accepted
        // aggregate. Completed publication deliberately ignores this input and
        // keeps its existing lossless lifecycle event path.
        (self.caption_reporter)(snapshot.clone());
        let Some(publisher) = publisher else {
            return;
        };

        match publisher.observe_snapshot(&snapshot) {
            Ok(PublisherSubmitOutcome::Handled) => {}
            Ok(PublisherSubmitOutcome::Closed) => {
                if !self.is_hard_stopped() {
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

    fn close_caption_session_at_boundary(&self) -> AppResult<()> {
        let (_output_gate, gate_was_poisoned) = match self.output_gate.lock() {
            Ok(gate) => (gate, false),
            Err(poisoned) => (poisoned.into_inner(), true),
        };
        let close_result = self.caption_session.close_generation(self.generation_id);
        if let Ok(Some(snapshot)) = &close_result {
            (self.caption_reporter)(snapshot.clone());
        }

        if gate_was_poisoned {
            let close_note = match close_result {
                Ok(_) => " Caption-session shutdown was still recorded.".to_string(),
                Err(error) => format!(" Caption-session shutdown also failed: {error}"),
            };
            return Err(AppError::state(format!(
                "Runtime generation lock was poisoned.{close_note}"
            )));
        }

        close_result.map(|_| ())
    }

    pub(crate) fn generation_id(&self) -> u64 {
        self.generation_id
    }

    fn stream_id(&self) -> &str {
        &self.stream_id
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
            runtime_plan,
            chatbox_pacer,
            caption_session,
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
        if !self.start_epoch_is_current(expected_stop_epoch) {
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        let generation = RuntimeGeneration::activate(&app, generation_id, caption_session)?;
        let publisher_init = initialize_runtime_publisher(
            &app,
            &config.osc,
            publication_policy,
            chatbox_pacer,
            generation.clone(),
        );
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

fn initialize_runtime_publisher(
    app: &AppHandle,
    config: &OscConfig,
    policy: ResolvedPublicationPolicy,
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
    let transport: Arc<dyn ChatboxTransport> = Arc::new(sender);
    let publisher = match policy {
        ResolvedPublicationPolicy::Completed => {
            let reporter_app = app.clone();
            let reporter: PublisherReporter = Arc::new(move |diagnostic| {
                emit_publisher_diagnostic(&reporter_app, diagnostic);
            });
            CompletedChatboxPublisher::start(transport, chatbox_pacer, generation, reporter)
                .map(RuntimeChatboxPublisher::Completed)
        }
        ResolvedPublicationPolicy::LiveUnit { .. } => {
            let generation_id = generation.generation_id();
            let reporter_app = app.clone();
            let reporter: LivePublisherReporter = Arc::new(move |diagnostic| {
                emit_live_publisher_diagnostic(&reporter_app, diagnostic);
            });
            LiveChatboxPublisher::start(
                transport,
                chatbox_pacer,
                generation_id,
                generation,
                policy,
                reporter,
            )
            .map(RuntimeChatboxPublisher::Live)
        }
    };

    match publisher {
        Ok(publisher) => RuntimePublisherInit::Ready(publisher),
        Err(error) => RuntimePublisherInit::Unavailable(error),
    }
}

fn run_runtime_thread(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
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

fn run_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
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

    run_openai_runtime(app, config, openai_api_key, publisher, generation)
}

fn run_openai_runtime(
    app: AppHandle,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
) -> AppResult<()> {
    if !generation.try_begin_work() {
        return Ok(());
    }

    let context = OpenAiRealtimeSessionContext {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
    };
    let mut recognition = connect_openai_realtime_session(
        context,
        config.stt.model,
        config.stt.languages.clone(),
        &openai_api_key,
    )?;
    if generation.is_hard_stopped() {
        recognition.stop()?;
        return Ok(());
    }

    let capture = open_input_capture(&config.audio)?;
    if generation.is_hard_stopped() {
        recognition.stop()?;
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
        recognition.stop()?;
        return Ok(());
    }

    let (recognition_sender, recognition_receiver) =
        sync_channel(RECOGNITION_COMMAND_QUEUE_CAPACITY);
    let recognition_worker = spawn_recognition_worker(
        app.clone(),
        publisher.clone(),
        generation.clone(),
        recognition,
        recognition_receiver,
    )?;
    let mut segmenter = new_recognition_segmenter(sample_rate);
    let stream = capture.stream;

    let runtime_result = (|| -> AppResult<()> {
        while !generation.is_work_cancelled() {
            let update = match receive_audio(&capture.receiver, RECEIVE_TIMEOUT)? {
                Some(samples) => segmenter.push_samples(samples, Instant::now()),
                None => segmenter.tick(Instant::now()),
            };
            apply_segmenter_update(
                &generation,
                &recognition_sender,
                segmenter.sample_rate(),
                update,
            )?;
        }

        Ok(())
    })();

    generation.cancel_work();
    drop(stream);
    let tail_speech_discarded = segmenter.finish().speech_ended;
    drop(recognition_sender);
    let worker_result = recognition_worker
        .join()
        .map_err(|_| AppError::runtime("Recognition worker thread panicked while stopping."))?;
    if tail_speech_discarded && !generation.is_hard_stopped() {
        emit_diagnostic(
            &app,
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
        (Ok(()), Err(worker_error)) if !generation.is_hard_stopped() => Err(worker_error),
        (Ok(()), Err(_)) | (Ok(()), Ok(())) => Ok(()),
    }
}

fn apply_segmenter_update(
    generation: &RuntimeGeneration,
    recognition_sender: &SyncSender<RecognitionCommand>,
    sample_rate_hz: u32,
    update: crate::segmenter::SegmenterUpdate,
) -> AppResult<()> {
    if update.speech_started {
        return send_recognition_command(
            generation,
            recognition_sender,
            RecognitionCommand::Start {
                unit_id: next_utterance_id("speech"),
                started_at_ms: now_ms(),
                sample_rate_hz,
                initial_audio: update.audio,
            },
        )
        .and_then(|()| {
            if update.speech_ended {
                send_recognition_command(
                    generation,
                    recognition_sender,
                    RecognitionCommand::EndInput,
                )
            } else {
                Ok(())
            }
        });
    }
    if !update.audio.is_empty() {
        send_recognition_command(
            generation,
            recognition_sender,
            RecognitionCommand::Audio {
                sample_rate_hz,
                samples: update.audio,
            },
        )?;
    }
    if update.speech_ended {
        send_recognition_command(generation, recognition_sender, RecognitionCommand::EndInput)?;
    }
    Ok(())
}

fn send_recognition_command(
    generation: &RuntimeGeneration,
    sender: &SyncSender<RecognitionCommand>,
    command: RecognitionCommand,
) -> AppResult<()> {
    match sender.try_send(command) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            generation.cancel_work();
            Err(AppError::stt_network(
                "OpenAI Realtime could not keep up with microphone audio; the bounded recognition queue filled without dropping audio.",
            ))
        }
        Err(TrySendError::Disconnected(_)) => {
            generation.cancel_work();
            Err(AppError::runtime(
                "Recognition worker stopped while microphone capture was still active.",
            ))
        }
    }
}

fn spawn_recognition_worker<S: RecognitionSession + 'static>(
    app: AppHandle,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    recognition: S,
    receiver: Receiver<RecognitionCommand>,
) -> AppResult<JoinHandle<AppResult<()>>> {
    thread::Builder::new()
        .name("vrc-live-caption-recognition".to_string())
        .spawn(move || run_recognition_worker(app, publisher, generation, recognition, receiver))
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
    mut recognition: impl RecognitionSession,
    receiver: Receiver<RecognitionCommand>,
) -> AppResult<()> {
    let work_result = (|| -> AppResult<()> {
        while !generation.is_work_cancelled() {
            match receiver.recv_timeout(RECOGNITION_EVENT_POLL_INTERVAL) {
                Ok(RecognitionCommand::Start {
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
                Ok(RecognitionCommand::Audio {
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

    generation.cancel_work();
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
        (Ok(()), Err(stop_error)) if !generation.is_hard_stopped() => Err(stop_error),
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
            if generation.is_hard_stopped() {
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
                "Dropped the oldest unstarted caption unit {unit_id} as one complete {page_count}-page publication because the Chatbox backlog was full. The App caption remains available."
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
                "Rejected caption unit {unit_id} as one complete {page_count}-page publication because it could not fit safely within the bounded Chatbox backlog. No partial pages were queued; the App caption remains available."
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
                "Discarded unstarted caption unit {unit_id} as one complete {page_count}-page publication after it exceeded the provisional backlog age. The App caption remains available."
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

fn emit_live_publisher_diagnostic<R: Runtime>(
    app: &AppHandle<R>,
    diagnostic: LivePublisherDiagnostic,
) {
    let update = match diagnostic {
        LivePublisherDiagnostic::ViewPublished {
            stream_id,
            unit_id,
            revision,
            byte_count,
            target,
        } => DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.live_view_sent",
            "Live caption view published",
            format!(
                "Published revision {revision} for {} in {stream_id} to {target} using {byte_count} encoded byte(s).",
                unit_id.as_deref().unwrap_or("the unitless stream")
            ),
        ),
        LivePublisherDiagnostic::ViewSendFailed {
            stream_id,
            unit_id,
            revision,
            error,
        } => DiagnosticUpdate::error(
            DiagnosticCategory::Osc,
            "osc.live_view_send_failed",
            "Live caption view could not be published",
            format!(
                "Revision {revision} for {} in {stream_id} failed and was not retried: {error}",
                unit_id.as_deref().unwrap_or("the unitless stream")
            ),
        ),
        LivePublisherDiagnostic::LayoutFailed {
            stream_id,
            unit_id,
            revision,
            reason,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.live_layout_failed",
            "Live caption view could not be laid out for Chatbox",
            format!(
                "Revision {revision} for {} in {stream_id} was not published: {reason}",
                unit_id.as_deref().unwrap_or("the unitless stream")
            ),
        ),
        LivePublisherDiagnostic::DraftDiscardedOnClose { reason } => {
            let (code, message) = match reason {
                PublisherCloseReason::Stop => (
                    "osc.live_draft_discarded_on_stop",
                    "Pending Live caption discarded on Stop",
                ),
                PublisherCloseReason::RuntimeError => (
                    "osc.live_draft_discarded_on_error",
                    "Pending Live caption discarded after Runtime failure",
                ),
            };
            DiagnosticUpdate::info(
                DiagnosticCategory::Osc,
                code,
                message,
                "The newest unsent Live revision was discarded; the App caption remains available.",
            )
        }
        LivePublisherDiagnostic::TypingFailed { error } => DiagnosticUpdate::error(
            DiagnosticCategory::Osc,
            "osc.live_typing_failed",
            "Live Chatbox typing indicator could not update",
            error.to_string(),
        ),
        LivePublisherDiagnostic::WorkerFailed { reason } => DiagnosticUpdate::error(
            DiagnosticCategory::Osc,
            "osc.live_publisher_failed",
            "Live Chatbox publisher stopped unexpectedly",
            reason,
        ),
    };

    emit_diagnostic(app, update);
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
