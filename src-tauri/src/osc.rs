use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const OSC_CHATBOX_INPUT_ADDRESS: &str = "/chatbox/input";
pub(crate) const OSC_TEST_MESSAGE: &str = "VRC Live Caption OSC test.";
const CHATBOX_WIDTH_PX: f32 = 280.0;
const CHATBOX_MAX_LINES: usize = 9;

pub(crate) struct OscSendResult {
    pub(crate) target: String,
    pub(crate) byte_count: usize,
    pub(crate) rendered_text: String,
    pub(crate) clipped: bool,
}

pub(crate) fn send_chatbox_osc(config: &OscConfig, text: &str) -> AppResult<OscSendResult> {
    let shaped = shape_chatbox_text(text);

    send_rendered_chatbox_osc(config, &shaped.text).map(|mut result| {
        result.clipped = shaped.clipped;
        result.rendered_text = shaped.text;
        result
    })
}

pub(crate) fn send_paced_chatbox_osc(
    config: &OscConfig,
    text: &str,
    last_send: &mut Option<Instant>,
) -> AppResult<OscSendResult> {
    if let Some(last_send) = last_send {
        let elapsed = last_send.elapsed();
        let minimum_interval = Duration::from_millis(config.min_interval_ms);

        if elapsed < minimum_interval {
            thread::sleep(minimum_interval - elapsed);
        }
    }

    let result = send_chatbox_osc(config, text)?;
    *last_send = Some(Instant::now());

    Ok(result)
}

fn send_rendered_chatbox_osc(config: &OscConfig, text: &str) -> AppResult<OscSendResult> {
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
        rendered_text: text.to_string(),
        clipped: false,
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

struct ShapedChatboxText {
    text: String,
    clipped: bool,
}

fn shape_chatbox_text(text: &str) -> ShapedChatboxText {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0.0;
    let mut clipped = false;

    for grapheme in normalized.graphemes(true) {
        let width = estimate_chatbox_width_px(grapheme);

        if !current_line.is_empty() && current_width + width > CHATBOX_WIDTH_PX {
            lines.push(current_line.trim_end().to_string());
            current_line.clear();
            current_width = 0.0;

            if lines.len() >= CHATBOX_MAX_LINES {
                clipped = true;
                break;
            }

            if grapheme.trim().is_empty() {
                continue;
            }
        }

        current_line.push_str(grapheme);
        current_width += width;
    }

    if !current_line.trim().is_empty() && lines.len() < CHATBOX_MAX_LINES {
        lines.push(current_line.trim_end().to_string());
    } else if !current_line.trim().is_empty() {
        clipped = true;
    }

    ShapedChatboxText {
        text: lines.join("\n"),
        clipped,
    }
}

fn estimate_chatbox_width_px(grapheme: &str) -> f32 {
    if grapheme.trim().is_empty() {
        return 5.0;
    }

    let mut width = 0.0;

    for character in grapheme.chars() {
        width += if character.is_ascii_punctuation() {
            5.0
        } else if character.is_ascii() {
            9.5
        } else if is_cjk_or_full_width(character) {
            18.0
        } else if character.is_alphanumeric() {
            9.5
        } else {
            18.0
        };
    }

    width
}

fn is_cjk_or_full_width(character: char) -> bool {
    matches!(
        character as u32,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
    )
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

    #[test]
    fn chatbox_text_is_clipped_to_nine_visible_lines() {
        let text = "中".repeat(144);
        let shaped = shape_chatbox_text(&text);

        assert!(shaped.clipped);
        assert_eq!(shaped.text.lines().count(), CHATBOX_MAX_LINES);
        assert!(shaped.text.lines().all(|line| line.chars().count() == 15));
    }

    #[test]
    fn chatbox_text_keeps_latin_lines_within_width_budget() {
        let text = "x".repeat(40);
        let shaped = shape_chatbox_text(&text);
        let lines: Vec<_> = shaped.text.lines().collect();

        assert_eq!(lines[0].chars().count(), 29);
        assert_eq!(lines[1].chars().count(), 11);
    }
}
