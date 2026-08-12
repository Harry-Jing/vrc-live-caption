//! Closed facade over the policy-specific Chatbox publishers.
//!
//! Runtime forwards each application-authoritative `CaptionAggregateUpdate`—the
//! newest full aggregate paired with its exact accepted change—without learning
//! which publisher owns the selected publication timing. Live consumes the full
//! aggregate; Completed consumes the exact change. Each concrete publisher
//! remains the sole owner of worker state, diagnostics, and publication behavior.

mod common;
mod completed;
mod completed_content;
mod diagnostics;
mod layout;
mod live;
mod osc;
mod pacer;
mod transport;

pub(crate) use common::{PublisherCloseReason, PublisherSubmitOutcome};
pub(crate) use osc::{ChatboxOscSender, OSC_CHATBOX_INPUT_ADDRESS, OSC_TEST_MESSAGE};
pub(crate) use pacer::ChatboxPacer;
pub(crate) use transport::{ChatboxSendReceipt, ChatboxTransport};

use crate::caption::CaptionAggregateUpdate;
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::config::{ContentSelection, OscConfig};
use crate::error::{AppError, AppResult};
use crate::events::DiagnosticUpdate;
use crate::generation_fence::GenerationCommitter;
use crate::host_resolver::HostResolver;
use completed::{CompletedChatboxPublisher, CompletedPublisherReporter};
use completed_content::CompletedContentCoordinator;
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
    stream_id: String,
    highest_update_revision: Arc<AtomicU64>,
    close_reason: Arc<AtomicU8>,
    committer: GenerationCommitter,
    reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    publisher: ChatboxPublisher,
}

#[derive(Clone)]
enum ChatboxPublisher {
    Completed(CompletedPublication),
    Live(LiveChatboxPublisher),
}

#[derive(Clone)]
enum CompletedPublication {
    Source(CompletedChatboxPublisher),
    Translation(CompletedContentCoordinator),
}

impl CompletedPublication {
    fn try_observe(&self, update: &CaptionAggregateUpdate) -> AppResult<PublisherSubmitOutcome> {
        match self {
            Self::Source(publisher) => publisher.try_observe(update),
            Self::Translation(publisher) => publisher.try_observe(update),
        }
    }

    fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        match self {
            Self::Source(publisher) => publisher.request_close(reason),
            Self::Translation(publisher) => publisher.request_close(reason),
        }
    }

    fn join(&self) -> AppResult<()> {
        match self {
            Self::Source(publisher) => publisher.join(),
            Self::Translation(publisher) => publisher.join(),
        }
    }
}

pub(crate) enum ChatboxPublicationInit {
    Disabled,
    Ready(ChatboxPublication),
    Unavailable(AppError),
}

#[derive(Clone, Copy)]
pub(crate) struct ChatboxPublicationPolicy {
    timing: ResolvedPublicationTiming,
    content: ContentSelection,
}

impl ChatboxPublicationPolicy {
    pub(crate) const fn new(timing: ResolvedPublicationTiming, content: ContentSelection) -> Self {
        Self { timing, content }
    }
}

pub(crate) struct ChatboxPublicationStart<'a> {
    pub(crate) config: &'a OscConfig,
    pub(crate) policy: ChatboxPublicationPolicy,
    pub(crate) pacer: ChatboxPacer,
    pub(crate) generation_id: u64,
    pub(crate) stream_id: &'a str,
    pub(crate) committer: GenerationCommitter,
    pub(crate) host_resolver: &'a HostResolver,
    pub(crate) is_cancelled: &'a dyn Fn() -> bool,
    pub(crate) reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
}

impl ChatboxPublication {
    pub(crate) fn initialize(start: ChatboxPublicationStart<'_>) -> ChatboxPublicationInit {
        let ChatboxPublicationStart {
            config,
            policy,
            pacer,
            generation_id,
            stream_id,
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
        match Self::start_with_transport_for_content(
            Arc::new(sender),
            pacer,
            generation_id,
            stream_id.to_string(),
            committer,
            policy,
            reporter,
        ) {
            Ok(publication) => ChatboxPublicationInit::Ready(publication),
            Err(error) => ChatboxPublicationInit::Unavailable(error),
        }
    }

    #[cfg(test)]
    pub(crate) fn start_with_transport(
        transport: Arc<dyn ChatboxTransport>,
        pacer: ChatboxPacer,
        generation_id: u64,
        committer: GenerationCommitter,
        timing: ResolvedPublicationTiming,
        reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    ) -> AppResult<Self> {
        Self::start_with_transport_for_content(
            transport,
            pacer,
            generation_id,
            format!("recognition-{generation_id}-1"),
            committer,
            ChatboxPublicationPolicy::new(timing, ContentSelection::SourceOnly),
            reporter,
        )
    }

    pub(crate) fn start_with_transport_for_content(
        transport: Arc<dyn ChatboxTransport>,
        pacer: ChatboxPacer,
        generation_id: u64,
        stream_id: String,
        committer: GenerationCommitter,
        policy: ChatboxPublicationPolicy,
        reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
    ) -> AppResult<Self> {
        let ChatboxPublicationPolicy { timing, content } = policy;
        let publication_committer = committer.clone();
        let publisher = match timing {
            ResolvedPublicationTiming::Completed => {
                let diagnostic_reporter = Arc::clone(&reporter);
                let completed_reporter: CompletedPublisherReporter = Arc::new(move |diagnostic| {
                    diagnostic_reporter(completed_publisher_diagnostic(diagnostic));
                });
                let completed = CompletedChatboxPublisher::start(
                    transport,
                    pacer,
                    committer,
                    completed_reporter,
                )?;
                let completed = match content {
                    ContentSelection::SourceOnly => CompletedPublication::Source(completed),
                    ContentSelection::TranslationOnly | ContentSelection::Bilingual => {
                        CompletedPublication::Translation(CompletedContentCoordinator::new(
                            content,
                            generation_id,
                            stream_id.clone(),
                            completed,
                        )?)
                    }
                };
                ChatboxPublisher::Completed(completed)
            }
            ResolvedPublicationTiming::LiveUnit { .. } => {
                if content != ContentSelection::SourceOnly {
                    return Err(AppError::config(
                        "Live publication supports Source content only.",
                    ));
                }
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
            stream_id,
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
        if update
            .snapshot
            .active_stream
            .as_ref()
            .is_some_and(|active| active.stream_id != self.stream_id)
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
