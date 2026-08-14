//! Closed facade over the policy-specific Chatbox publishers.
//!
//! Runtime forwards each application-authoritative `CaptionAggregateUpdate`—the
//! newest full aggregate paired with its exact accepted change—without learning
//! which publisher owns the selected publication timing. Live consumes the full
//! aggregate; Completed consumes the exact change. Each concrete publisher
//! remains the sole owner of worker state, diagnostics, and publication behavior.

mod common;
mod completed;
mod diagnostics;
mod layout;
mod live;
mod osc;
mod text_pacing;
mod transport;

#[cfg(test)]
mod test_support;

use common::describe_layout_error;
pub(crate) use common::{PublicationObservationOutcome, PublisherCloseReason};
pub(crate) use layout::PreparedChatboxText;
use layout::prepare_single_message;
use osc::ChatboxOscSender;
pub(crate) use osc::OSC_CHATBOX_INPUT_ADDRESS;
pub(crate) use text_pacing::ChatboxTextPacer;
pub(crate) use transport::{ChatboxSendReceipt, ChatboxTransport};

use crate::caption::CaptionAggregateUpdate;
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use crate::events::DiagnosticUpdate;
use crate::generation_fence::GenerationCommitter;
use crate::host_resolver::HostResolver;
use completed::{CompletedChatboxPublisher, CompletedPublisherReporter};
use diagnostics::{
    completed_publisher_diagnostic, completed_update_discarded_after_close,
    live_publisher_diagnostic, live_update_discarded_after_close,
};
use live::{LiveChatboxPublisher, LivePublisherReporter};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

const CLOSE_REASON_NONE: u8 = 0;
const CLOSE_REASON_RUNTIME_ERROR: u8 = 1;
const CLOSE_REASON_STOP: u8 = 2;
const OSC_TEST_TEXT: &str = "VRC Live Caption OSC test.";

#[derive(Clone)]
pub(crate) struct ChatboxPublication {
    generation_id: u64,
    highest_snapshot_revision: Arc<AtomicU64>,
    close_reason: Arc<AtomicU8>,
    committer: GenerationCommitter,
    reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    worker: PublicationWorker,
}

#[derive(Clone)]
enum PublicationWorker {
    Completed(CompletedChatboxPublisher),
    Live(LiveChatboxPublisher),
}

pub(crate) enum ChatboxPublicationStartOutcome {
    Disabled,
    Ready(ChatboxPublication),
    Unavailable(AppError),
}

pub(crate) struct ChatboxPublicationStartRequest<'a> {
    pub(crate) config: &'a OscConfig,
    pub(crate) publication_timing: ResolvedPublicationTiming,
    pub(crate) text_pacer: ChatboxTextPacer,
    pub(crate) generation_id: u64,
    pub(crate) committer: GenerationCommitter,
    pub(crate) host_resolver: &'a HostResolver,
    pub(crate) is_cancelled: &'a dyn Fn() -> bool,
    pub(crate) reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
}

pub(crate) fn send_osc_test_message(
    config: &OscConfig,
    text_pacer: &ChatboxTextPacer,
    host_resolver: &HostResolver,
) -> AppResult<ChatboxSendReceipt> {
    let sender = ChatboxOscSender::new(config, host_resolver, &|| false)?;
    let text = prepare_single_message(OSC_TEST_TEXT)
        .map_err(|error| {
            AppError::state(format!(
                "OSC test message could not be prepared: {}",
                describe_layout_error(error)
            ))
        })?
        .ok_or_else(|| AppError::state("OSC test message must not be empty."))?;
    text_pacer
        .wait_for_text_attempt(None)?
        .ok_or_else(|| AppError::state("OSC Test pacing was cancelled."))?
        .attempt(|| sender.send_text(&text))
}

impl ChatboxPublication {
    pub(crate) fn start(
        request: ChatboxPublicationStartRequest<'_>,
    ) -> ChatboxPublicationStartOutcome {
        let ChatboxPublicationStartRequest {
            config,
            publication_timing,
            text_pacer,
            generation_id,
            committer,
            host_resolver,
            is_cancelled,
            reporter,
        } = request;
        if !config.enabled {
            return ChatboxPublicationStartOutcome::Disabled;
        }

        let sender = match ChatboxOscSender::new(config, host_resolver, is_cancelled) {
            Ok(sender) => sender,
            Err(error) => return ChatboxPublicationStartOutcome::Unavailable(error),
        };
        match Self::start_with_transport(
            Arc::new(sender),
            text_pacer,
            generation_id,
            committer,
            publication_timing,
            reporter,
        ) {
            Ok(publication) => ChatboxPublicationStartOutcome::Ready(publication),
            Err(error) => ChatboxPublicationStartOutcome::Unavailable(error),
        }
    }

    pub(crate) fn start_with_transport(
        transport: Arc<dyn ChatboxTransport>,
        text_pacer: ChatboxTextPacer,
        generation_id: u64,
        committer: GenerationCommitter,
        publication_timing: ResolvedPublicationTiming,
        reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    ) -> AppResult<Self> {
        let publication_committer = committer.clone();
        let worker = match publication_timing {
            ResolvedPublicationTiming::Completed => {
                let diagnostic_reporter = Arc::clone(&reporter);
                let completed_reporter: CompletedPublisherReporter = Arc::new(move |diagnostic| {
                    diagnostic_reporter(completed_publisher_diagnostic(diagnostic));
                });
                PublicationWorker::Completed(CompletedChatboxPublisher::start(
                    transport,
                    text_pacer,
                    committer,
                    completed_reporter,
                )?)
            }
            ResolvedPublicationTiming::LiveUnit { .. } => {
                let diagnostic_reporter = Arc::clone(&reporter);
                let live_reporter: LivePublisherReporter = Arc::new(move |diagnostic| {
                    diagnostic_reporter(live_publisher_diagnostic(diagnostic));
                });
                PublicationWorker::Live(LiveChatboxPublisher::start(
                    transport,
                    text_pacer,
                    generation_id,
                    committer,
                    publication_timing,
                    live_reporter,
                )?)
            }
        };
        Ok(Self {
            generation_id,
            highest_snapshot_revision: Arc::new(AtomicU64::new(0)),
            close_reason: Arc::new(AtomicU8::new(CLOSE_REASON_NONE)),
            committer: publication_committer,
            reporter,
            worker,
        })
    }

    pub(crate) fn try_observe(
        &self,
        update: &CaptionAggregateUpdate,
    ) -> AppResult<PublicationObservationOutcome> {
        if update
            .snapshot
            .active_stream
            .as_ref()
            .is_some_and(|active| active.generation != self.generation_id)
        {
            return Ok(PublicationObservationOutcome::Handled);
        }
        // The aggregate revision is the internal idempotency key. A publication
        // observes each strictly newer accepted update at most once; exact
        // replays and out-of-order delivery are successful no-ops.
        if self
            .highest_snapshot_revision
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                (update.snapshot.snapshot_revision > current)
                    .then_some(update.snapshot.snapshot_revision)
            })
            .is_err()
        {
            return Ok(PublicationObservationOutcome::Handled);
        }

        let outcome = match &self.worker {
            PublicationWorker::Completed(publisher) => publisher.try_observe(update),
            PublicationWorker::Live(publisher) => publisher.try_observe(&update.snapshot),
        }?;
        if outcome == PublicationObservationOutcome::Closed {
            self.report_discarded_after_close(update);
        }
        Ok(outcome)
    }

    pub(crate) fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        self.record_close_reason(reason);
        match &self.worker {
            PublicationWorker::Completed(publisher) => publisher.request_close(reason),
            PublicationWorker::Live(publisher) => publisher.request_close(reason),
        }
    }

    pub(crate) fn join(&self) -> AppResult<()> {
        match &self.worker {
            PublicationWorker::Completed(publisher) => publisher.join(),
            PublicationWorker::Live(publisher) => publisher.join(),
        }
    }

    fn record_close_reason(&self, reason: PublisherCloseReason) {
        match reason {
            PublisherCloseReason::Stop => {
                self.close_reason.store(CLOSE_REASON_STOP, Ordering::SeqCst);
            }
            PublisherCloseReason::RuntimeError => {
                let _ = self.close_reason.compare_exchange(
                    CLOSE_REASON_NONE,
                    CLOSE_REASON_RUNTIME_ERROR,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
        }
    }

    fn report_discarded_after_close(&self, update: &CaptionAggregateUpdate) {
        let reason = if self.committer.is_stop_requested() {
            PublisherCloseReason::Stop
        } else {
            match self.close_reason.load(Ordering::SeqCst) {
                CLOSE_REASON_STOP => PublisherCloseReason::Stop,
                CLOSE_REASON_NONE | CLOSE_REASON_RUNTIME_ERROR => {
                    PublisherCloseReason::RuntimeError
                }
                _ => PublisherCloseReason::RuntimeError,
            }
        };
        let diagnostic = match &self.worker {
            PublicationWorker::Completed(_) => {
                completed_update_discarded_after_close(update, reason)
            }
            PublicationWorker::Live(_) => live_update_discarded_after_close(reason),
        };
        if let Some(diagnostic) = diagnostic {
            (self.reporter)(diagnostic);
        }
    }
}

#[cfg(test)]
#[path = "chatbox/chatbox_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "chatbox/regression_tests.rs"]
mod regression_tests;
