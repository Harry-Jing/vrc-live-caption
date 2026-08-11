//! Generation-scoped output admission and Chatbox publication ownership.
//!
//! This module owns the hard Stop fence, work cancellation, caption-aggregate
//! admission, linearized App/Chatbox commits, and publication lifecycle.

use crate::caption::{
    CaptionAggregateSnapshotV2, CaptionAggregateStore, CaptionAggregateUpdate, CaptionSnapshotV2,
};
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::chatbox::{
    ChatboxPacer, ChatboxPublication, ChatboxPublicationStart, PublisherCloseReason,
    PublisherSubmitOutcome,
};
use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, emit_caption_aggregate_changed, emit_diagnostic,
};
use crate::generation_fence::{GenerationCommitter, GenerationFence};
use crate::host_resolver::HostResolver;
use crate::recognition::{RecognitionEvent, RecognitionUnitAbortReason};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Runtime};

pub(super) use crate::chatbox::ChatboxPublicationInit;

type CaptionAggregateReporter = Arc<dyn Fn(CaptionAggregateSnapshotV2) + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RuntimeGeneration {
    generation_id: u64,
    stream_id: String,
    caption_aggregate: CaptionAggregateStore,
    caption_reporter: CaptionAggregateReporter,
    generation_fence: GenerationFence,
    work_cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecognitionEventSubmitOutcome {
    Accepted,
    Ignored,
    Stopped,
}

impl RuntimeGeneration {
    pub(super) fn activate<R: Runtime>(
        app: &AppHandle<R>,
        generation_id: u64,
        caption_aggregate: CaptionAggregateStore,
    ) -> AppResult<Self> {
        let snapshot = caption_aggregate.begin_generation(generation_id)?;
        let active = snapshot.active_stream.as_ref().ok_or_else(|| {
            AppError::state("Caption aggregate rejected a non-monotonic runtime generation.")
        })?;
        if active.generation != generation_id {
            return Err(AppError::state(
                "Caption aggregate activated a different runtime generation.",
            ));
        }
        let stream_id = active.stream_id.clone();

        let reporter_app = app.clone();
        let caption_reporter: CaptionAggregateReporter = Arc::new(move |snapshot| {
            emit_caption_aggregate_changed(&reporter_app, snapshot);
        });
        caption_reporter(snapshot);

        Ok(Self {
            generation_id,
            stream_id,
            caption_aggregate,
            caption_reporter,
            generation_fence: GenerationFence::new(),
            work_cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    #[cfg(test)]
    pub(crate) fn active() -> Self {
        let caption_aggregate = CaptionAggregateStore::default();
        if let Err(error) = caption_aggregate.begin_generation(1) {
            tracing::error!(error_message = %error, "test caption aggregate could not start");
        }

        Self {
            generation_id: 1,
            stream_id: "recognition-1-1".to_string(),
            caption_aggregate,
            caption_reporter: Arc::new(|_| {}),
            generation_fence: GenerationFence::new(),
            work_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(super) fn poison_commit_gate_for_test(&self) {
        self.generation_fence.poison_for_test();
    }

    pub(crate) fn request_stop(&self, publication: Option<&ChatboxPublication>) -> AppResult<()> {
        // Establish one shared App/Chatbox commit cutoff before cancelling
        // capture and recognition work or waiting for either output sink. A commit
        // that already crossed the fence may finish; every later one is rejected.
        self.generation_fence.request_stop();
        self.cancel_work();
        self.close_outputs_at_boundary(publication, PublisherCloseReason::Stop)
    }

    pub(super) fn close_publication_at_boundary(
        &self,
        publication: Option<&ChatboxPublication>,
        reason: PublisherCloseReason,
    ) -> AppResult<()> {
        // Close generation-wide commit admission before asking Chatbox to close.
        // This ordering also protects RuntimeError cleanup, which has no hard-Stop
        // marker of its own. An already-linearized transport call may finish; the
        // gate below waits for it before returning.
        self.generation_fence.close_admission();
        let close_result = match publication {
            Some(publication) => publication.request_close(reason),
            None => Ok(()),
        };

        // Recover the guard even after a panic poisoned the gate. Normal
        // commits already fail closed on poison, but Chatbox admission must
        // still close so Stop cannot hang while joining an idle worker.
        if let Err(gate_error) = self.generation_fence.wait_for_commits() {
            let close_note = match close_result {
                Ok(()) => " Chatbox shutdown was still requested.".to_string(),
                Err(error) => format!(" Chatbox shutdown also failed: {error}"),
            };
            Err(AppError::state(format!("{gate_error}{close_note}")))
        } else {
            close_result
        }
    }

    pub(super) fn close_outputs_for_runtime_error(
        &self,
        publication: Option<&ChatboxPublication>,
    ) -> AppResult<()> {
        self.close_outputs_at_boundary(publication, PublisherCloseReason::RuntimeError)
    }

    fn close_outputs_at_boundary(
        &self,
        publication: Option<&ChatboxPublication>,
        reason: PublisherCloseReason,
    ) -> AppResult<()> {
        // Close and wake Chatbox admission before waiting for any commit that
        // already crossed the generation fence. The aggregate closes only
        // after those commits finish; worker join happens at the caller.
        let publication_result = self.close_publication_at_boundary(publication, reason);
        let aggregate_result = self.close_caption_aggregate_at_boundary();

        match (publication_result, aggregate_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(publication_error), Err(aggregate_error)) => Err(AppError::state(format!(
                "Runtime outputs could not close cleanly: {publication_error} {aggregate_error}"
            ))),
        }
    }

    pub(crate) fn commit_if_active(&self, commit: impl FnOnce()) -> AppResult<bool> {
        self.try_commit(commit).map(|result| result.is_some())
    }

    pub(crate) fn try_commit<T>(&self, commit: impl FnOnce() -> T) -> AppResult<Option<T>> {
        self.committer().try_commit(commit)
    }

    pub(crate) fn committer(&self) -> GenerationCommitter {
        self.generation_fence.committer()
    }

    fn start_caption_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: String,
        started_at_ms: u64,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        self.caption_aggregate
            .start_unit(generation, stream_id, unit_id, started_at_ms)
    }

    fn abort_source_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: &str,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        self.caption_aggregate
            .abort_source_unit(generation, stream_id, unit_id)
    }

    fn accept_caption(
        &self,
        caption: CaptionSnapshotV2,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        self.caption_aggregate.accept_caption(caption)
    }

    fn caption_snapshot(&self) -> AppResult<CaptionAggregateSnapshotV2> {
        self.caption_aggregate.snapshot()
    }

    fn report_caption_snapshot(&self, snapshot: CaptionAggregateSnapshotV2) {
        (self.caption_reporter)(snapshot);
    }

    fn close_caption_aggregate_at_boundary(&self) -> AppResult<()> {
        let gate_result = self.generation_fence.wait_for_commits();
        let close_result = self.caption_aggregate.close_generation(self.generation_id);
        if let Ok(Some(snapshot)) = &close_result {
            (self.caption_reporter)(snapshot.clone());
        }

        if let Err(gate_error) = gate_result {
            let close_note = match close_result {
                Ok(_) => " Caption-aggregate shutdown was still recorded.".to_string(),
                Err(error) => format!(" Caption-aggregate shutdown also failed: {error}"),
            };
            return Err(AppError::state(format!("{gate_error}{close_note}")));
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
        self.generation_fence.is_stop_requested()
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
        publisher: Option<&ChatboxPublication>,
        event: RecognitionEvent,
    ) -> AppResult<RecognitionEventSubmitOutcome> {
        let Some(submit_result) =
            self.try_commit(|| self.submit_recognition_event_at_boundary(app, publisher, event))?
        else {
            return Ok(RecognitionEventSubmitOutcome::Stopped);
        };
        submit_result
    }

    fn submit_recognition_event_at_boundary<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&ChatboxPublication>,
        event: RecognitionEvent,
    ) -> AppResult<RecognitionEventSubmitOutcome> {
        match event {
            RecognitionEvent::UnitStarted {
                generation,
                stream_id,
                unit_id,
                started_at_ms,
            } => {
                let Some(update) =
                    self.start_caption_unit(generation, &stream_id, unit_id, started_at_ms)?
                else {
                    return Ok(RecognitionEventSubmitOutcome::Ignored);
                };
                self.report_accepted_update(app, publisher, update);
            }
            RecognitionEvent::UnitAborted {
                generation,
                stream_id,
                unit_id,
                reason,
            } => {
                let failure_detail = match reason {
                    RecognitionUnitAbortReason::NoSpeech => None,
                    RecognitionUnitAbortReason::Failed { detail } => Some(detail),
                };
                let Some(update) = self.abort_source_unit(generation, &stream_id, &unit_id)? else {
                    return Ok(RecognitionEventSubmitOutcome::Ignored);
                };
                self.report_accepted_update(app, publisher, update);
                if let Some(detail) = failure_detail {
                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::error(
                            DiagnosticCategory::Recognition,
                            "stt.item_failed",
                            "One caption unit could not be transcribed",
                            detail,
                        ),
                    );
                }
            }
            RecognitionEvent::Caption(caption) => {
                let Some(update) = self.accept_caption(caption)? else {
                    return Ok(RecognitionEventSubmitOutcome::Ignored);
                };
                self.report_accepted_update(app, publisher, update);
            }
        }

        Ok(RecognitionEventSubmitOutcome::Accepted)
    }

    pub(super) fn abort_open_source_units_for_reconnect<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&ChatboxPublication>,
    ) -> AppResult<()> {
        self.fail_open_source_units(
            app,
            publisher,
            "Speech was discarded because the recognition connection was interrupted.",
        )
    }

    pub(super) fn abort_open_source_units_for_terminal_failure<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&ChatboxPublication>,
    ) -> AppResult<()> {
        self.fail_open_source_units(
            app,
            publisher,
            "Speech was discarded because recognition stopped with a terminal error.",
        )
    }

    fn fail_open_source_units<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&ChatboxPublication>,
        detail: &str,
    ) -> AppResult<()> {
        let snapshot = self.caption_snapshot()?;
        for open_source_unit in snapshot.open_source_units {
            let _submit_outcome = self.submit_recognition_event(
                app,
                publisher,
                RecognitionEvent::UnitAborted {
                    generation: self.generation_id(),
                    stream_id: self.stream_id().to_string(),
                    unit_id: open_source_unit.unit_id,
                    reason: RecognitionUnitAbortReason::Failed {
                        detail: detail.to_string(),
                    },
                },
            )?;
        }
        Ok(())
    }

    fn report_accepted_update<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        publisher: Option<&ChatboxPublication>,
        update: CaptionAggregateUpdate,
    ) {
        // The App and Chatbox publication observe the exact same store-accepted
        // update. The facade owns all timing-specific interpretation.
        self.report_caption_snapshot(update.snapshot.clone());
        let Some(publisher) = publisher else {
            return;
        };

        match publisher.try_submit(&update) {
            Ok(PublisherSubmitOutcome::Handled) => {}
            Ok(PublisherSubmitOutcome::Closed) => {}
            Err(error) => emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Chatbox snapshot could not be observed"),
            ),
        }
    }
}

pub(super) fn initialize_chatbox_publication<R: Runtime>(
    app: &AppHandle<R>,
    config: &OscConfig,
    timing: ResolvedPublicationTiming,
    chatbox_pacer: ChatboxPacer,
    generation: &RuntimeGeneration,
    host_resolver: &HostResolver,
    is_cancelled: &dyn Fn() -> bool,
) -> ChatboxPublicationInit {
    let reporter_app = app.clone();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> =
        Arc::new(move |diagnostic| emit_diagnostic(&reporter_app, diagnostic));
    ChatboxPublication::initialize(ChatboxPublicationStart {
        config,
        timing,
        pacer: chatbox_pacer,
        generation_id: generation.generation_id(),
        committer: generation.committer(),
        host_resolver,
        is_cancelled,
        reporter,
    })
}

pub(super) fn finish_runtime_output<R: Runtime>(
    app: &AppHandle<R>,
    generation: &RuntimeGeneration,
    publication: Option<&ChatboxPublication>,
    reason: PublisherCloseReason,
) {
    let close_result = match reason {
        PublisherCloseReason::Stop => generation.request_stop(publication),
        PublisherCloseReason::RuntimeError => {
            generation.close_outputs_for_runtime_error(publication)
        }
    };
    if let Err(error) = close_result {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Chatbox publication could not close"),
        );
    }

    if let Some(publication) = publication
        && let Err(error) = publication.join()
    {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Chatbox publication failed while closing"),
        );
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
