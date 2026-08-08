//! Closed-enum runtime facade over the policy-specific Chatbox publishers.
//!
//! Runtime code can forward both normalized caption-session snapshots and the
//! existing Completed lifecycle events without learning which publisher owns
//! the selected publication policy. The inactive input is deliberately a
//! handled no-op; each concrete publisher remains the sole owner of its worker
//! state and publication behavior.

use crate::caption_session::CaptionSessionSnapshotV1;
use crate::chatbox_publisher::{CompletedChatboxPublisher, CompletedPublisherEvent};
use crate::chatbox_publisher_common::{PublisherCloseReason, PublisherSubmitOutcome};
use crate::error::AppResult;
use crate::live_chatbox_publisher::LiveChatboxPublisher;
use crate::runtime_generation::ChatboxPublisherBoundary;

#[derive(Clone)]
pub(crate) enum RuntimeChatboxPublisher {
    Completed(CompletedChatboxPublisher),
    Live(LiveChatboxPublisher),
}

impl RuntimeChatboxPublisher {
    pub(crate) fn observe_snapshot(
        &self,
        snapshot: &CaptionSessionSnapshotV1,
    ) -> AppResult<PublisherSubmitOutcome> {
        match self {
            Self::Completed(_) => Ok(PublisherSubmitOutcome::Handled),
            Self::Live(publisher) => publisher.try_observe(snapshot),
        }
    }

    pub(crate) fn try_submit_completed_event(
        &self,
        event: CompletedPublisherEvent,
    ) -> AppResult<PublisherSubmitOutcome> {
        match self {
            Self::Completed(publisher) => publisher.try_submit(event),
            Self::Live(_) => Ok(PublisherSubmitOutcome::Handled),
        }
    }

    pub(crate) fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        match self {
            Self::Completed(publisher) => publisher.request_close(reason),
            Self::Live(publisher) => publisher.request_close(reason),
        }
    }

    pub(crate) fn join(&self) -> AppResult<()> {
        match self {
            Self::Completed(publisher) => publisher.join(),
            Self::Live(publisher) => publisher.join(),
        }
    }
}

impl ChatboxPublisherBoundary for RuntimeChatboxPublisher {
    fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        Self::request_close(self, reason)
    }
}

// Completed publisher tests exercise the generation boundary directly. Keep
// that seam usable without wrapping the concrete worker or changing its state.
impl ChatboxPublisherBoundary for CompletedChatboxPublisher {
    fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        Self::request_close(self, reason)
    }
}

#[cfg(test)]
#[path = "chatbox_publication_tests.rs"]
mod tests;
