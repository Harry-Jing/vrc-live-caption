use super::RuntimeGeneration;
use crate::chatbox::{
    ChatboxPacer, ChatboxSendReceipt, ChatboxTransport, CompletedChatboxPublisher,
    PublisherReporter, RuntimeChatboxPublisher,
};
use crate::error::{AppError, AppResult};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct RecordingChatboxTransport {
    pub(super) text_sender: std::sync::mpsc::Sender<String>,
    pub(super) typing_sender: Option<std::sync::mpsc::Sender<bool>>,
}

impl ChatboxTransport for RecordingChatboxTransport {
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        self.text_sender.send(text.to_string()).map_err(|_| {
            AppError::osc_send(
                "runtime test transport",
                "Text receiver disconnected.".to_string(),
            )
        })?;

        Ok(ChatboxSendReceipt {
            target: "runtime-test".to_string(),
            byte_count: text.len(),
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
) -> AppResult<(RuntimeChatboxPublisher, std::sync::mpsc::Receiver<String>)> {
    let (text_sender, text_receiver) = std::sync::mpsc::channel();
    let reporter: PublisherReporter = Arc::new(|_| {});
    let publisher = CompletedChatboxPublisher::start(
        Arc::new(RecordingChatboxTransport {
            text_sender,
            typing_sender,
        }),
        ChatboxPacer::default(),
        generation,
        reporter,
    )?;

    Ok((RuntimeChatboxPublisher::Completed(publisher), text_receiver))
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
