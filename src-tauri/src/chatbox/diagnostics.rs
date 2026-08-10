//! UI-facing diagnostic mapping for Chatbox publisher events.
//!
//! Concrete publishers report policy-specific facts without depending on the
//! Tauri event contract. This module translates those facts into the stable
//! diagnostic vocabulary consumed by the runtime UI.

use super::common::PublisherCloseReason;
use super::completed::PublisherDiagnostic;
use super::live::LivePublisherDiagnostic;
use crate::events::{DiagnosticCategory, DiagnosticUpdate};

pub(crate) fn completed_publisher_diagnostic(diagnostic: PublisherDiagnostic) -> DiagnosticUpdate {
    match diagnostic {
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
    }
}

pub(crate) fn live_publisher_diagnostic(diagnostic: LivePublisherDiagnostic) -> DiagnosticUpdate {
    match diagnostic {
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
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
