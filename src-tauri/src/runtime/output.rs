//! Generation-scoped output admission and Chatbox publication ownership.
//!
//! This module owns the hard Stop fence, work cancellation, caption-session
//! admission, linearized App/Chatbox commits, and publisher lifecycle.

use crate::capability_planner::ResolvedPublicationPolicy;
use crate::caption_session::{CaptionSessionSnapshotV1, CaptionSessionStore, CaptionSnapshotV1};
use crate::chatbox::{
    ChatboxOscSender, ChatboxPacer, ChatboxTransport, CompletedChatboxPublisher,
    CompletedPublisherEvent, LiveChatboxPublisher, LivePublisherReporter, PublisherCloseReason,
    PublisherReporter, PublisherSubmitOutcome, RuntimeChatboxPublisher,
    completed_publisher_diagnostic, live_publisher_diagnostic,
};
use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, UtteranceEndReason, emit_caption_session_changed,
    emit_diagnostic, emit_utterance_ended, emit_utterance_started, now_ms,
};
use crate::host_resolver::HostResolver;
use crate::recognition::{RecognitionEndReason, RecognitionEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Runtime};

type CaptionSessionReporter = Arc<dyn Fn(CaptionSessionSnapshotV1) + Send + Sync>;

pub(crate) trait ChatboxPublisherBoundary {
    fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()>;
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

pub(super) enum RuntimePublisherInit {
    Disabled,
    Ready(RuntimeChatboxPublisher),
    Unavailable(AppError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecognitionEventSubmitOutcome {
    Accepted,
    Ignored,
    Stopped,
}

pub(super) fn publisher_boundary(
    publisher: Option<&RuntimeChatboxPublisher>,
) -> Option<&dyn ChatboxPublisherBoundary> {
    publisher.map(|publisher| publisher as &dyn ChatboxPublisherBoundary)
}

pub(super) fn publisher_failure_message<'a>(
    publisher: Option<&RuntimeChatboxPublisher>,
    completed: &'a str,
    live: &'a str,
) -> &'a str {
    match publisher {
        Some(RuntimeChatboxPublisher::Live(_)) => live,
        Some(RuntimeChatboxPublisher::Completed(_)) | None => completed,
    }
}

impl RuntimeGeneration {
    pub(super) fn activate<R: Runtime>(
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

    #[cfg(test)]
    pub(super) fn test_output_gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.output_gate)
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

    pub(super) fn close_publisher_at_boundary(
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

    fn start_caption_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: String,
        started_at_ms: u64,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        self.caption_session
            .start_unit(generation, stream_id, unit_id, started_at_ms)
    }

    fn end_caption_unit_without_caption(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: &str,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        self.caption_session
            .end_unit_without_caption(generation, stream_id, unit_id)
    }

    fn accept_caption(
        &self,
        caption: CaptionSnapshotV1,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        self.caption_session.accept_caption(caption)
    }

    fn caption_snapshot(&self) -> AppResult<CaptionSessionSnapshotV1> {
        self.caption_session.snapshot()
    }

    fn report_caption_snapshot(&self, snapshot: CaptionSessionSnapshotV1) {
        (self.caption_reporter)(snapshot);
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

    pub(super) fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub(super) fn cancel_work(&self) {
        self.work_cancelled.store(true, Ordering::SeqCst);
    }

    pub(super) fn is_work_cancelled(&self) -> bool {
        self.work_cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn is_hard_stop_requested(&self) -> bool {
        self.hard_stop_requested.load(Ordering::SeqCst)
    }

    pub(super) fn accepts_new_work(&self) -> bool {
        // These loads are the work-submission decision point. If they win the
        // race with Stop, the request is in flight and may finish, but its
        // result still has to pass commit_if_active.
        !self.is_work_cancelled() && !self.is_hard_stop_requested()
    }

    pub(super) fn submit_recognition_event<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        event: RecognitionEvent,
    ) -> AppResult<RecognitionEventSubmitOutcome> {
        let mut submit_result = None;
        let committed = self.commit_if_active(|| {
            submit_result = Some(self.submit_recognition_event_at_boundary(app, publisher, event));
        })?;
        if !committed {
            return Ok(RecognitionEventSubmitOutcome::Stopped);
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
    ) -> AppResult<RecognitionEventSubmitOutcome> {
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
                    return Ok(RecognitionEventSubmitOutcome::Ignored);
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
                    return Ok(RecognitionEventSubmitOutcome::Ignored);
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
                    return Ok(RecognitionEventSubmitOutcome::Ignored);
                };
                self.report_accepted_snapshot(app, publisher, snapshot);

                if is_completed && let (Some(publisher), Some(unit_id)) = (publisher, unit_id) {
                    submit_completed_chatbox_candidate(app, publisher, self, unit_id, text);
                }
            }
        }

        Ok(RecognitionEventSubmitOutcome::Accepted)
    }

    pub(super) fn abort_active_units_for_reconnect<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
    ) -> AppResult<()> {
        self.fail_active_units(
            app,
            publisher,
            "Speech was discarded because the recognition connection was interrupted.",
        )
    }

    pub(super) fn abort_active_units_for_terminal_failure<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
    ) -> AppResult<()> {
        self.fail_active_units(
            app,
            publisher,
            "Speech was discarded because recognition stopped with a terminal error.",
        )
    }

    fn fail_active_units<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&RuntimeChatboxPublisher>,
        detail: &str,
    ) -> AppResult<()> {
        let snapshot = self.caption_snapshot()?;
        for active_unit in snapshot.active_units {
            let _submit_outcome = self.submit_recognition_event(
                app,
                publisher,
                RecognitionEvent::UnitEnded {
                    generation: self.generation_id(),
                    stream_id: self.stream_id().to_string(),
                    unit_id: active_unit.unit_id,
                    reason: RecognitionEndReason::Failed {
                        detail: detail.to_string(),
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

pub(super) fn initialize_runtime_publisher<R: Runtime>(
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

pub(super) fn finish_runtime_output<R: Runtime>(
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
