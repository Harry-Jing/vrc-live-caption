use super::RuntimeGeneration;
use crate::caption::{
    CAPTION_AGGREGATE_CONTRACT_VERSION, CaptionAggregateChange, CaptionAggregateSnapshot,
    CaptionAggregateUpdate,
};
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::chatbox::{
    ChatboxPublication, ChatboxSendReceipt, ChatboxTextPacer, ChatboxTransport, PreparedChatboxText,
};
use crate::error::{AppError, AppResult};
use crate::events::DiagnosticUpdate;
use std::sync::Arc;
use std::time::Duration;

pub(super) struct RecordingChatboxTransport {
    pub(super) text_sender: std::sync::mpsc::Sender<String>,
    pub(super) typing_sender: Option<std::sync::mpsc::Sender<bool>>,
}

impl ChatboxTransport for RecordingChatboxTransport {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.text_sender
            .send(text.as_str().to_string())
            .map_err(|_| {
                AppError::osc_send(
                    "runtime test transport",
                    "Text receiver disconnected.".to_string(),
                )
            })?;

        Ok(ChatboxSendReceipt {
            target: "runtime-test".to_string(),
            byte_count: text.as_str().len(),
        })
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        if let Some(sender) = &self.typing_sender {
            sender.send(is_typing).map_err(|_| {
                AppError::osc_send(
                    "runtime test transport",
                    "Typing receiver disconnected.".to_string(),
                )
            })?;
        }
        Ok(())
    }
}

pub(super) fn runtime_test_publisher(
    generation: RuntimeGeneration,
    typing_sender: Option<std::sync::mpsc::Sender<bool>>,
) -> AppResult<(ChatboxPublication, std::sync::mpsc::Receiver<String>)> {
    let (text_sender, text_receiver) = std::sync::mpsc::channel();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(|_| {});
    let publication = ChatboxPublication::start_with_transport(
        Arc::new(RecordingChatboxTransport {
            text_sender,
            typing_sender,
        }),
        ChatboxTextPacer::default(),
        generation.generation_id(),
        generation.committer(),
        ResolvedPublicationTiming::Completed,
        reporter,
    )?;

    Ok((publication, text_receiver))
}

pub(super) fn inactive_caption_update(revision: u64) -> CaptionAggregateUpdate {
    CaptionAggregateUpdate {
        snapshot: CaptionAggregateSnapshot {
            contract_version: CAPTION_AGGREGATE_CONTRACT_VERSION,
            snapshot_revision: revision,
            active_stream: None,
            open_source_units: Vec::new(),
            captions: Vec::new(),
            translation_units: Vec::new(),
        },
        change: CaptionAggregateChange::SourceUnitAborted {
            unit_id: "inactive-unit".to_string(),
        },
    }
}

pub(super) fn receive_json_event(
    receiver: &std::sync::mpsc::Receiver<String>,
    event_name: &str,
) -> AppResult<serde_json::Value> {
    let payload = receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
        AppError::runtime(format!("Did not receive the expected {event_name} event."))
    })?;

    serde_json::from_str(&payload).map_err(|error| {
        AppError::runtime(format!("Failed to parse the {event_name} event: {error}"))
    })
}
