use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use std::net::UdpSocket;

pub(crate) const OSC_CHATBOX_INPUT_ADDRESS: &str = "/chatbox/input";
pub(crate) const OSC_TEST_MESSAGE: &str = "VRC Live Caption OSC test.";

pub(crate) struct OscSendResult {
    pub(crate) target: String,
    pub(crate) byte_count: usize,
}

pub(crate) fn send_chatbox_osc(config: &OscConfig, text: &str) -> AppResult<OscSendResult> {
    let target = format!("{}:{}", config.host, config.port);
    let packet = chatbox_input_packet(text);
    let packet_bytes =
        encoder::encode(&packet).map_err(|error| AppError::osc_encode(error.to_string()))?;
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|error| AppError::osc_bind(error.to_string()))?;
    let sent = socket
        .send_to(&packet_bytes, &target)
        .map_err(|error| AppError::osc_send(&target, error.to_string()))?;

    if sent != packet_bytes.len() {
        return Err(AppError::osc_send_incomplete(
            &target,
            packet_bytes.len(),
            sent,
        ));
    }

    Ok(OscSendResult {
        target,
        byte_count: sent,
    })
}

fn chatbox_input_packet(text: &str) -> OscPacket {
    OscPacket::Message(OscMessage {
        addr: OSC_CHATBOX_INPUT_ADDRESS.to_string(),
        args: vec![
            OscType::String(text.to_string()),
            OscType::Bool(true),
            OscType::Bool(false),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatbox_test_packet_sends_final_text_immediately_without_notification_sound() {
        assert_eq!(
            chatbox_input_packet("test"),
            OscPacket::Message(OscMessage {
                addr: OSC_CHATBOX_INPUT_ADDRESS.to_string(),
                args: vec![
                    OscType::String("test".to_string()),
                    OscType::Bool(true),
                    OscType::Bool(false),
                ],
            })
        );
    }

    #[test]
    fn chatbox_test_packet_can_be_encoded() {
        let packet = chatbox_input_packet(OSC_TEST_MESSAGE);

        assert!(encoder::encode(&packet).is_ok());
    }
}
