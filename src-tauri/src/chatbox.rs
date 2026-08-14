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
mod pacer;
mod transport;

pub(crate) use common::{PublisherCloseReason, PublisherSubmitOutcome, describe_layout_error};
pub(crate) use layout::{PreparedChatboxText, prepare_single_message};
pub(crate) use osc::{ChatboxOscSender, OSC_CHATBOX_INPUT_ADDRESS, OSC_TEST_MESSAGE};
pub(crate) use pacer::ChatboxPacer;
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

#[derive(Clone)]
pub(crate) struct ChatboxPublication {
    generation_id: u64,
    highest_update_revision: Arc<AtomicU64>,
    close_reason: Arc<AtomicU8>,
    committer: GenerationCommitter,
    reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    publisher: ChatboxPublisher,
}

#[derive(Clone)]
enum ChatboxPublisher {
    Completed(CompletedChatboxPublisher),
    Live(LiveChatboxPublisher),
}

pub(crate) enum ChatboxPublicationInit {
    Disabled,
    Ready(ChatboxPublication),
    Unavailable(AppError),
}

pub(crate) struct ChatboxPublicationStart<'a> {
    pub(crate) config: &'a OscConfig,
    pub(crate) timing: ResolvedPublicationTiming,
    pub(crate) pacer: ChatboxPacer,
    pub(crate) generation_id: u64,
    pub(crate) committer: GenerationCommitter,
    pub(crate) host_resolver: &'a HostResolver,
    pub(crate) is_cancelled: &'a dyn Fn() -> bool,
    pub(crate) reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
}

impl ChatboxPublication {
    pub(crate) fn initialize(start: ChatboxPublicationStart<'_>) -> ChatboxPublicationInit {
        let ChatboxPublicationStart {
            config,
            timing,
            pacer,
            generation_id,
            committer,
            host_resolver,
            is_cancelled,
            reporter,
        } = start;
        if !config.enabled {
            return ChatboxPublicationInit::Disabled;
        }

        let sender = match ChatboxOscSender::new(config, host_resolver, is_cancelled) {
            Ok(sender) => sender,
            Err(error) => return ChatboxPublicationInit::Unavailable(error),
        };
        match Self::start_with_transport(
            Arc::new(sender),
            pacer,
            generation_id,
            committer,
            timing,
            reporter,
        ) {
            Ok(publication) => ChatboxPublicationInit::Ready(publication),
            Err(error) => ChatboxPublicationInit::Unavailable(error),
        }
    }

    pub(crate) fn start_with_transport(
        transport: Arc<dyn ChatboxTransport>,
        pacer: ChatboxPacer,
        generation_id: u64,
        committer: GenerationCommitter,
        timing: ResolvedPublicationTiming,
        reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    ) -> AppResult<Self> {
        let publication_committer = committer.clone();
        let publisher = match timing {
            ResolvedPublicationTiming::Completed => {
                let diagnostic_reporter = Arc::clone(&reporter);
                let completed_reporter: CompletedPublisherReporter = Arc::new(move |diagnostic| {
                    diagnostic_reporter(completed_publisher_diagnostic(diagnostic));
                });
                ChatboxPublisher::Completed(CompletedChatboxPublisher::start(
                    transport,
                    pacer,
                    committer,
                    completed_reporter,
                )?)
            }
            ResolvedPublicationTiming::LiveUnit { .. } => {
                let diagnostic_reporter = Arc::clone(&reporter);
                let live_reporter: LivePublisherReporter = Arc::new(move |diagnostic| {
                    diagnostic_reporter(live_publisher_diagnostic(diagnostic));
                });
                ChatboxPublisher::Live(LiveChatboxPublisher::start(
                    transport,
                    pacer,
                    generation_id,
                    committer,
                    timing,
                    live_reporter,
                )?)
            }
        };
        Ok(Self {
            generation_id,
            highest_update_revision: Arc::new(AtomicU64::new(0)),
            close_reason: Arc::new(AtomicU8::new(CLOSE_REASON_NONE)),
            committer: publication_committer,
            reporter,
            publisher,
        })
    }

    pub(crate) fn try_submit(
        &self,
        update: &CaptionAggregateUpdate,
    ) -> AppResult<PublisherSubmitOutcome> {
        if update
            .snapshot
            .active_stream
            .as_ref()
            .is_some_and(|active| active.generation != self.generation_id)
        {
            return Ok(PublisherSubmitOutcome::Handled);
        }
        // The aggregate revision is the internal idempotency key. A publication
        // observes each strictly newer accepted update at most once; exact
        // replays and out-of-order delivery are successful no-ops.
        if self
            .highest_update_revision
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                (update.snapshot.snapshot_revision > current)
                    .then_some(update.snapshot.snapshot_revision)
            })
            .is_err()
        {
            return Ok(PublisherSubmitOutcome::Handled);
        }

        let outcome = match &self.publisher {
            ChatboxPublisher::Completed(publisher) => publisher.try_observe(update),
            ChatboxPublisher::Live(publisher) => publisher.try_observe(&update.snapshot),
        }?;
        if outcome == PublisherSubmitOutcome::Closed {
            self.report_discarded_after_close(update);
        }
        Ok(outcome)
    }

    pub(crate) fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        self.record_close_reason(reason);
        match &self.publisher {
            ChatboxPublisher::Completed(publisher) => publisher.request_close(reason),
            ChatboxPublisher::Live(publisher) => publisher.request_close(reason),
        }
    }

    pub(crate) fn join(&self) -> AppResult<()> {
        match &self.publisher {
            ChatboxPublisher::Completed(publisher) => publisher.join(),
            ChatboxPublisher::Live(publisher) => publisher.join(),
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
        let diagnostic = match &self.publisher {
            ChatboxPublisher::Completed(_) => {
                completed_update_discarded_after_close(update, reason)
            }
            ChatboxPublisher::Live(_) => live_update_discarded_after_close(reason),
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
