//! Runtime lifecycle for Phase 1 outgoing captions.
//!
//! The capture loop drains microphone samples and never performs blocking STT
//! upload work. Completed speech segments are sent to a bounded STT worker queue;
//! per-segment STT or OSC failures emit diagnostics and keep the runtime alive.
//! Startup failures such as invalid config or unavailable microphone remain fatal.
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
use crate::capability_planner::{MOCK_BOUNDED_MODEL, MOCK_ONGOING_ONLY_MODEL, RuntimePlanSnapshot};
use crate::caption_session::{
    CaptionLane, CaptionSessionSnapshotV1, CaptionSessionStore, CaptionSnapshotV1,
};
use crate::chatbox_pacer::ChatboxPacer;
use crate::chatbox_publisher::{
    CompletedChatboxPublisher, CompletedPublisherEvent, PublisherCloseReason, PublisherDiagnostic,
    PublisherReporter, PublisherSubmitOutcome,
};
use crate::config::{AppConfig, OscConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, UtteranceEndReason,
    emit_caption_session_changed, emit_diagnostic, emit_status, emit_utterance_ended,
    emit_utterance_started, next_utterance_id, now_ms,
};
use crate::openai_bounded::{CompletedAudioUnit, OpenAiBoundedOutcome, OpenAiBoundedSession};
use crate::osc::ChatboxOscSender;
use crate::recognition_fakes::{
    FakeBoundedRecognitionAdapter, FakeOngoingCompletedRecognitionAdapter,
    FakeOngoingOnlyRecognitionAdapter, RecognitionEvent, ScriptedRecognitionContext, ScriptedText,
};
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeCredentialSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase,
    RuntimeSessionSnapshot,
};
use crate::segmenter::SpeechSegmenter;
use crate::stt::build_stt_client;
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
    pub(crate) runtime_plan: RuntimePlanSnapshot,
    pub(crate) chatbox_pacer: ChatboxPacer,
    pub(crate) caption_session: CaptionSessionStore,
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
    generation_id: u64,
    stream_id: String,
    caption_session: CaptionSessionStore,
    caption_reporter: CaptionSessionReporter,
    output_gate: Arc<Mutex<()>>,
    hard_stop_requested: Arc<AtomicBool>,
    work_cancelled: Arc<AtomicBool>,
}

struct ActiveSpeechUnit {
    unit_id: String,
    started_at_ms: u64,
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

    fn start_caption_unit<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        unit_id: String,
        started_at_ms: u64,
    ) -> AppResult<bool> {
        let _output_gate = self
            .output_gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation lock was poisoned."))?;
        if self.hard_stop_requested.load(Ordering::SeqCst) {
            return Ok(false);
        }

        let Some(snapshot) = self.caption_session.start_unit(
            self.generation_id,
            &self.stream_id,
            unit_id.clone(),
            started_at_ms,
        )?
        else {
            return Ok(false);
        };
        (self.caption_reporter)(snapshot);
        emit_utterance_started(
            app,
            self.generation_id,
            self.stream_id.clone(),
            unit_id,
            started_at_ms,
        );

        Ok(true)
    }

    fn accept_caption(&self, caption: CaptionSnapshotV1) -> AppResult<bool> {
        let _output_gate = self
            .output_gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation lock was poisoned."))?;
        if self.hard_stop_requested.load(Ordering::SeqCst) {
            return Ok(false);
        }

        let Some(snapshot) = self.caption_session.accept_caption(caption)? else {
            return Ok(false);
        };
        (self.caption_reporter)(snapshot);

        Ok(true)
    }

    pub(crate) fn submit_recognition_event<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&CompletedChatboxPublisher>,
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
                (self.caption_reporter)(snapshot);
                emit_utterance_started(app, generation, stream_id, unit_id.clone(), started_at_ms);

                if let Some(publisher) = publisher
                    && let Err(error) =
                        publisher.try_submit(CompletedPublisherEvent::Started { unit_id })
                {
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::from_error(&error, "Chatbox activity could not start"),
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
                (self.caption_reporter)(snapshot);

                if is_completed && let (Some(publisher), Some(unit_id)) = (publisher, unit_id) {
                    submit_completed_chatbox_candidate(app, publisher, self, unit_id, text);
                }
            }
        }

        Ok(true)
    }

    fn end_caption_unit<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        unit_id: String,
        reason: UtteranceEndReason,
    ) -> AppResult<bool> {
        let _output_gate = self
            .output_gate
            .lock()
            .map_err(|_| AppError::state("Runtime generation lock was poisoned."))?;
        if self.hard_stop_requested.load(Ordering::SeqCst) {
            return Ok(false);
        }

        let Some(snapshot) = self.caption_session.end_unit_without_caption(
            self.generation_id,
            &self.stream_id,
            &unit_id,
        )?
        else {
            return Ok(false);
        };
        (self.caption_reporter)(snapshot);
        emit_utterance_ended(
            app,
            self.generation_id,
            self.stream_id.clone(),
            unit_id,
            reason,
            now_ms(),
        );

        Ok(true)
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

    fn generation_id(&self) -> u64 {
        self.generation_id
    }

    fn stream_id(&self) -> &str {
        &self.stream_id
    }

    fn next_unitless_source_revision(&self) -> AppResult<u64> {
        let current_revision = self
            .caption_session
            .snapshot()?
            .captions
            .into_iter()
            .find(|caption| {
                caption.generation == self.generation_id
                    && caption.stream_id == self.stream_id
                    && caption.unit_id.is_none()
                    && caption.lane == CaptionLane::Source
            })
            .map(|caption| caption.revision)
            .unwrap_or(0);

        current_revision.checked_add(1).ok_or_else(|| {
            AppError::state("Mock unitless recognition revision counter was exhausted.")
        })
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

    pub(crate) fn emit_mock_transcript<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        language: &str,
        model: &str,
    ) -> AppResult<()> {
        let (generation, publisher) = {
            let guard = self
                .handle
                .lock()
                .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
            let handle = guard.as_ref().ok_or_else(|| {
                AppError::runtime("Mock Transcript requires an active runtime generation.")
            })?;
            (handle.generation.clone(), handle.publisher.clone())
        };
        let unit_id = next_utterance_id("mock");
        let started_at_ms = now_ms();
        let context = ScriptedRecognitionContext {
            generation: generation.generation_id(),
            stream_id: generation.stream_id().to_string(),
            language: Some(language.to_string()),
            provider: "mock".to_string(),
            model: model.to_string(),
        };
        let events = match model {
            MOCK_BOUNDED_MODEL => FakeBoundedRecognitionAdapter::new(context).script_completed(
                unit_id,
                started_at_ms,
                ScriptedText::new(
                    "Testing bounded caption preview from the mock runtime.",
                    now_ms(),
                ),
            ),
            MOCK_ONGOING_ONLY_MODEL => {
                let first_revision = generation.next_unitless_source_revision()?;
                FakeOngoingOnlyRecognitionAdapter::new(context).script_stream_from(
                    first_revision,
                    &[
                        ScriptedText::new("Testing live caption preview...", now_ms()),
                        ScriptedText::new(
                            "Testing live caption preview from the ongoing-only mock runtime.",
                            now_ms(),
                        ),
                    ],
                )
            }
            _ => FakeOngoingCompletedRecognitionAdapter::new(context).script_unit(
                unit_id,
                started_at_ms,
                &[ScriptedText::new(
                    "Testing live caption preview...",
                    now_ms(),
                )],
                ScriptedText::new(
                    "Testing live caption preview from the mock runtime.",
                    now_ms(),
                ),
            ),
        };

        for event in events {
            if !generation.submit_recognition_event(app, publisher.as_ref(), event)? {
                return Err(AppError::state(
                    "Mock recognition event was rejected by the active generation.",
                ));
            }
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
            runtime_plan,
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
    let mut active_unit: Option<ActiveSpeechUnit> = None;
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
                        &mut active_unit,
                        &segment_sender,
                        publisher.as_ref(),
                        &generation,
                    )?;
                }
                continue;
            };

            let update = segmenter.push_samples(samples, Instant::now());

            if update.speech_started {
                let next_utterance = next_utterance_id("speech");
                let started_at_ms = now_ms();
                if !generation.start_caption_unit(&app, next_utterance.clone(), started_at_ms)? {
                    return Ok(());
                }
                start_chatbox_activity(&app, publisher.as_ref(), &next_utterance);
                active_unit = Some(ActiveSpeechUnit {
                    unit_id: next_utterance,
                    started_at_ms,
                });
            }

            if let Some(samples) = update.ready_segment {
                queue_speech_segment(
                    &app,
                    segmenter.sample_rate(),
                    samples,
                    &mut active_unit,
                    &segment_sender,
                    publisher.as_ref(),
                    &generation,
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
        if let Some(active_unit) = active_unit {
            end_utterance_without_final(
                &app,
                publisher.as_ref(),
                &generation,
                active_unit.unit_id,
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
    segment_sender: SyncSender<CompletedAudioUnit>,
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
    active_unit: &mut Option<ActiveSpeechUnit>,
    segment_sender: &SyncSender<CompletedAudioUnit>,
    publisher: Option<&CompletedChatboxPublisher>,
    generation: &RuntimeGeneration,
) -> AppResult<()> {
    // The segmenter only yields segments that reached the voiced minimum, and
    // crossing the voiced minimum announces the utterance first, so an id is
    // always present here. A missing id means segmentation broke that
    // invariant, and silently minting a fresh id would send the UI a
    // caption for an utterance that was never announced.
    let active_unit = active_unit.take().ok_or_else(|| {
        AppError::runtime("Speech segment was ready without an announced utterance.")
    })?;

    match segment_sender.try_send(CompletedAudioUnit {
        unit_id: active_unit.unit_id,
        started_at_ms: active_unit.started_at_ms,
        sample_rate_hz: sample_rate,
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
                        segment.samples.len() as f32 / segment.sample_rate_hz as f32
                    ),
                ),
            );
            end_utterance_without_final(
                app,
                publisher,
                generation,
                segment.unit_id,
                UtteranceEndReason::Discarded,
            );

            Ok(())
        }
        Err(TrySendError::Disconnected(segment)) => {
            end_utterance_without_final(
                app,
                publisher,
                generation,
                segment.unit_id,
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
    segment_receiver: Receiver<CompletedAudioUnit>,
    generation: RuntimeGeneration,
) -> AppResult<JoinHandle<()>> {
    thread::Builder::new()
        .name("vrc-live-caption-stt-worker".to_string())
        .spawn(move || {
            let bounded_session = OpenAiBoundedSession::new(
                generation.generation_id(),
                generation.stream_id().to_string(),
                config.stt.clone(),
                http_client,
                openai_api_key,
            );
            run_stt_worker(
                app,
                config,
                publisher,
                segment_receiver,
                generation,
                move |unit| bounded_session.recognize(unit),
            )
        })
        .map_err(|error| AppError::runtime(format!("Failed to start STT worker thread: {error}")))
}

fn run_stt_worker<R: Runtime>(
    app: AppHandle<R>,
    config: AppConfig,
    publisher: Option<CompletedChatboxPublisher>,
    segment_receiver: Receiver<CompletedAudioUnit>,
    generation: RuntimeGeneration,
    recognize: impl Fn(&CompletedAudioUnit) -> AppResult<OpenAiBoundedOutcome>,
) {
    let mut discarded_segments: usize = 0;
    while let Ok(segment) = segment_receiver.recv() {
        if generation.is_work_cancelled() {
            discarded_segments += 1;
            end_utterance_without_final(
                &app,
                publisher.as_ref(),
                &generation,
                segment.unit_id,
                UtteranceEndReason::Discarded,
            );
            continue;
        }

        if let Err(error) = transcribe_and_emit_final(
            &app,
            &config,
            segment,
            publisher.as_ref(),
            &generation,
            &recognize,
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

fn transcribe_and_emit_final<R: Runtime>(
    app: &AppHandle<R>,
    config: &AppConfig,
    segment: CompletedAudioUnit,
    publisher: Option<&CompletedChatboxPublisher>,
    generation: &RuntimeGeneration,
    recognize: &impl Fn(&CompletedAudioUnit) -> AppResult<OpenAiBoundedOutcome>,
) -> AppResult<()> {
    if !generation.try_begin_work() {
        end_utterance_without_final(
            app,
            publisher,
            generation,
            segment.unit_id,
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
                segment.samples.len() as f32 / segment.sample_rate_hz as f32
            ),
        ),
    );

    let utterance_id = segment.unit_id.clone();
    let outcome = match recognize(&segment) {
        Ok(outcome) => outcome,
        Err(error) => {
            let committed = generation.end_caption_unit(
                app,
                utterance_id.clone(),
                UtteranceEndReason::SttFailed,
            )?;
            if !committed {
                discard_late_transcription_result(app, publisher, generation, utterance_id);
                return Ok(());
            }
            abort_chatbox_activity(app, publisher, &utterance_id);

            return Err(error);
        }
    };

    let caption = match outcome {
        OpenAiBoundedOutcome::NoSpeech => {
            let committed = generation.end_caption_unit(
                app,
                utterance_id.clone(),
                UtteranceEndReason::NoSpeech,
            )?;
            if !committed {
                discard_late_transcription_result(app, publisher, generation, utterance_id);
                return Ok(());
            }
            abort_chatbox_activity(app, publisher, &utterance_id);
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
        OpenAiBoundedOutcome::Completed(caption) => caption,
    };
    let text = caption.text.clone();
    let committed = generation.accept_caption(caption)?;

    if !committed {
        discard_late_transcription_result(app, publisher, generation, utterance_id);
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

    submit_completed_chatbox_candidate(app, publisher, generation, utterance_id, text);

    Ok(())
}

fn discard_late_transcription_result<R: Runtime>(
    app: &AppHandle<R>,
    publisher: Option<&CompletedChatboxPublisher>,
    generation: &RuntimeGeneration,
    utterance_id: String,
) {
    end_utterance_without_final(
        app,
        publisher,
        generation,
        utterance_id,
        UtteranceEndReason::Discarded,
    );
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
            "Runtime stop was requested before this caption could be sent.",
        ),
    );
}

fn submit_completed_chatbox_candidate<R: Runtime>(
    app: &AppHandle<R>,
    publisher: &CompletedChatboxPublisher,
    generation: &RuntimeGeneration,
    unit_id: String,
    text: String,
) {
    match publisher.try_submit(CompletedPublisherEvent::Completed { unit_id, text }) {
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
    generation: &RuntimeGeneration,
    utterance_id: String,
    reason: UtteranceEndReason,
) {
    let aggregate_resolved = match generation.end_caption_unit(app, utterance_id.clone(), reason) {
        Ok(resolved) => resolved,
        Err(error) => {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Caption unit could not resolve"),
            );
            false
        }
    };
    let resolution = NoFinalUtteranceResolution {
        utterance_id,
        reason,
    };

    if let Err(error) =
        complete_no_final_utterance(publisher, resolution, |utterance_id, reason| {
            if !aggregate_resolved {
                emit_utterance_ended(
                    app,
                    generation.generation_id(),
                    generation.stream_id().to_string(),
                    utterance_id,
                    reason,
                    now_ms(),
                );
            }
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
#[path = "runtime_tests.rs"]
mod tests;
