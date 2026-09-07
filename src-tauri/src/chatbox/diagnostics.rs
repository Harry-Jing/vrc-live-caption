//! UI-facing diagnostic mapping for Chatbox publisher events.
//!
//! Concrete publishers report policy-specific facts without depending on the
//! Tauri event contract. This module translates those facts into the stable
//! diagnostic vocabulary consumed by the runtime UI.

use super::common::PublisherCloseReason;
use super::completed::{CompletedPublisherDiagnostic, TranslationResolution};
use super::live::LivePublisherDiagnostic;
use crate::caption::{
    CaptionAggregateChange, CaptionAggregateUpdate, CaptionLane, CaptionState,
    TranslationFailureReason,
};
use crate::events::{DiagnosticCategory, DiagnosticUpdate};

fn translation_failure_code(reason: TranslationFailureReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "translation.failed".to_string())
}

fn describe_translation_resolution(resolution: &TranslationResolution) -> String {
    match resolution {
        TranslationResolution::Failed(reason) => {
            format!(
                "its Translation failed ({})",
                translation_failure_code(*reason)
            )
        }
        TranslationResolution::WaitExpired => {
            "no terminal Translation outcome arrived within the publisher's wait budget".to_string()
        }
        TranslationResolution::LayoutFailed { reason } => {
            format!("the bilingual pair could not be laid out for Chatbox: {reason}")
        }
    }
}

pub(super) fn completed_update_discarded_after_close(
    update: &CaptionAggregateUpdate,
    reason: PublisherCloseReason,
) -> Option<DiagnosticUpdate> {
    let CaptionAggregateChange::CaptionAccepted(caption) = &update.change else {
        return None;
    };
    if caption.lane != CaptionLane::Source
        || caption.state != CaptionState::Completed
        || caption.unit_id.is_none()
    {
        return None;
    }

    Some(match reason {
        PublisherCloseReason::Stop => DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.send_skipped_on_stop",
            "Chatbox send skipped",
            "Runtime stop was requested before this caption could be sent.",
        ),
        PublisherCloseReason::RuntimeError => DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.completed_unit_discarded_after_close",
            "Completed Chatbox publication discarded",
            "The runtime output worker closed before this completed caption could enter its queue. The App caption remains available.",
        ),
    })
}

pub(super) fn live_update_discarded_after_close(
    reason: PublisherCloseReason,
) -> Option<DiagnosticUpdate> {
    match reason {
        PublisherCloseReason::Stop => None,
        PublisherCloseReason::RuntimeError => Some(DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.live_snapshot_discarded_after_close",
            "Live Chatbox snapshot discarded",
            "The Live publisher closed before this accepted App caption snapshot could be observed.",
        )),
    }
}

pub(crate) fn completed_publisher_diagnostic(
    diagnostic: CompletedPublisherDiagnostic,
) -> DiagnosticUpdate {
    match diagnostic {
        CompletedPublisherDiagnostic::UnitSendSucceeded {
            unit_id,
            page_count,
            byte_count,
            target,
        } => DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.completed_unit_sent",
            "Completed caption sent to Chatbox OSC",
            format!(
                "Sent {page_count} ordered page(s) for {unit_id} to {target} using {byte_count} encoded byte(s)."
            ),
        ),
        CompletedPublisherDiagnostic::UnitDroppedOverload {
            unit_id,
            page_count,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_dropped_overload",
            "Completed caption dropped from Chatbox backlog",
            format!(
                "Dropped the oldest caption unit {unit_id} whose first send attempt had not started, preserving it as one complete {page_count}-page publication, because the Chatbox backlog was full. The App caption remains available."
            ),
        ),
        CompletedPublisherDiagnostic::UnitRejectedOverload {
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
        CompletedPublisherDiagnostic::UnitExpired {
            unit_id,
            page_count,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_expired",
            "Completed caption expired from Chatbox backlog",
            format!(
                "Discarded caption unit {unit_id}, whose first send attempt had not started, as one complete {page_count}-page publication after it exceeded the provisional backlog age. The App caption remains available."
            ),
        ),
        CompletedPublisherDiagnostic::LayoutFailed { unit_id, reason } => {
            DiagnosticUpdate::warning(
                DiagnosticCategory::Osc,
                "osc.completed_layout_failed",
                "Completed caption could not be laid out for Chatbox",
                format!("Caption unit {unit_id} was not sent: {reason}"),
            )
        }
        CompletedPublisherDiagnostic::UnitSendFailed {
            unit_id,
            page_index,
            page_count,
            pages_sent,
            error,
        } => DiagnosticUpdate::from_error(
            &error,
            format!(
                "Completed Chatbox send failed for {unit_id} on page {page_index} of {page_count} after {pages_sent} successful page(s); the failed page was not retried and the unit's remaining pages were discarded"
            ),
        ),
        CompletedPublisherDiagnostic::PagesDiscardedOnClose {
            reason,
            unit_count,
            page_count,
            send_started_unit_count,
            translation_wait_unit_count,
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
                    "Discarded {page_count} unsent page(s) across {unit_count} caption unit(s), including {send_started_unit_count} unit(s) whose first send attempt had begun and {translation_wait_unit_count} unit(s) still waiting for Translation."
                ),
            )
        }
        CompletedPublisherDiagnostic::UnitOmittedWithoutTranslation {
            unit_id,
            resolution,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_omitted_without_translation",
            "Completed caption omitted from Chatbox",
            format!(
                "Translation-only publication omitted caption unit {unit_id} because {}. The App caption remains available.",
                describe_translation_resolution(&resolution)
            ),
        ),
        CompletedPublisherDiagnostic::UnitQueuedWithoutTranslation {
            unit_id,
            resolution,
        } => DiagnosticUpdate::warning(
            DiagnosticCategory::Osc,
            "osc.completed_unit_queued_without_translation",
            "Completed caption queued without its Translation",
            format!(
                "Bilingual publication queued caption unit {unit_id} as Source only because {}. Later units still request Translation.",
                describe_translation_resolution(&resolution)
            ),
        ),
        CompletedPublisherDiagnostic::TypingFailed { is_typing, error } => {
            let transition = if is_typing { "on" } else { "off" };
            DiagnosticUpdate::from_error(
                &error,
                format!("Chatbox typing indicator could not turn {transition}"),
            )
        }
        CompletedPublisherDiagnostic::WorkerFailed { reason } => DiagnosticUpdate::error(
            DiagnosticCategory::Osc,
            "osc.completed_publisher_failed",
            "Completed Chatbox publisher stopped unexpectedly",
            reason,
        ),
    }
}

pub(crate) fn live_publisher_diagnostic(diagnostic: LivePublisherDiagnostic) -> DiagnosticUpdate {
    match diagnostic {
        LivePublisherDiagnostic::ViewportSendSucceeded {
            stream_id,
            unit_id,
            revision,
            byte_count,
            target,
        } => DiagnosticUpdate::info(
            DiagnosticCategory::Osc,
            "osc.live_view_sent",
            "Live caption viewport sent to Chatbox OSC",
            format!(
                "Sent revision {revision} for {} in {stream_id} to {target} using {byte_count} encoded byte(s).",
                unit_id.as_deref().unwrap_or("the unitless stream")
            ),
        ),
        LivePublisherDiagnostic::ViewportSendFailed {
            stream_id,
            unit_id,
            revision,
            error,
        } => DiagnosticUpdate::error(
            DiagnosticCategory::Osc,
            "osc.live_view_send_failed",
            "Live caption viewport could not be sent",
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
            "Live caption viewport could not be prepared for Chatbox",
            format!(
                "Revision {revision} for {} in {stream_id} was not sent: {reason}",
                unit_id.as_deref().unwrap_or("the unitless stream")
            ),
        ),
        LivePublisherDiagnostic::PendingViewportDiscardedOnClose { reason } => {
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
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
