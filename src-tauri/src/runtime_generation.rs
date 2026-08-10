//! One Start-to-Stop generation boundary shared by App and Chatbox output.
//!
//! This module owns the hard Stop fence, work cancellation, caption-session
//! admission, and linearized output commit. Runtime orchestration remains
//! responsible for mapping recognition events to concrete publisher inputs.

use crate::caption_session::{CaptionSessionSnapshotV1, CaptionSessionStore, CaptionSnapshotV1};
use crate::chatbox::PublisherCloseReason;
use crate::error::{AppError, AppResult};
use crate::events::emit_caption_session_changed;
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

impl RuntimeGeneration {
    pub(crate) fn activate<R: Runtime>(
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
    pub(crate) fn test_output_gate(&self) -> Arc<Mutex<()>> {
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

    pub(crate) fn close_publisher_at_boundary(
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

    pub(crate) fn start_caption_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: String,
        started_at_ms: u64,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        self.caption_session
            .start_unit(generation, stream_id, unit_id, started_at_ms)
    }

    pub(crate) fn end_caption_unit_without_caption(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: &str,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        self.caption_session
            .end_unit_without_caption(generation, stream_id, unit_id)
    }

    pub(crate) fn accept_caption(
        &self,
        caption: CaptionSnapshotV1,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        self.caption_session.accept_caption(caption)
    }

    pub(crate) fn caption_snapshot(&self) -> AppResult<CaptionSessionSnapshotV1> {
        self.caption_session.snapshot()
    }

    pub(crate) fn report_caption_snapshot(&self, snapshot: CaptionSessionSnapshotV1) {
        (self.caption_reporter)(snapshot);
    }

    pub(crate) fn close_caption_session_at_boundary(&self) -> AppResult<()> {
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

    pub(crate) fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub(crate) fn cancel_work(&self) {
        self.work_cancelled.store(true, Ordering::SeqCst);
    }

    pub(crate) fn is_work_cancelled(&self) -> bool {
        self.work_cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn is_hard_stop_requested(&self) -> bool {
        self.hard_stop_requested.load(Ordering::SeqCst)
    }

    pub(crate) fn accepts_new_work(&self) -> bool {
        // These loads are the work-submission decision point. If they win the
        // race with Stop, the request is in flight and may finish, but its
        // result still has to pass commit_if_active.
        !self.is_work_cancelled() && !self.is_hard_stop_requested()
    }
}
