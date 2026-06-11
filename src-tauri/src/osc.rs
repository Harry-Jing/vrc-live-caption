//! VRChat Chatbox output over OSC (UDP).
//!
//! Text is shaped to VRChat's fixed Chatbox layout before sending: lines wrap
//! against a pixel width budget and clip at the hard visible line cap. The
//! layout constants and per-character width estimates approximate the
//! measured TMP contract in `docs/research/vrchat-chatbox-reference.md`: a
//! 300px `ChatText` rectangle with 10px margins (280px of usable width) and
//! a 9-line cap.
//!
//! `ChatboxOscSender` owns one UDP socket for its lifetime and paces repeated
//! sends with a minimum interval so VRChat's Chatbox rate limit does not drop
//! updates.

use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const OSC_CHATBOX_INPUT_ADDRESS: &str = "/chatbox/input";
pub(crate) const OSC_TEST_MESSAGE: &str = "VRC Live Caption OSC test.";
// ChatText rectangle minus margins; see the module docs for the source.
const CHATBOX_WIDTH_PX: f32 = 280.0;
const CHATBOX_MAX_LINES: usize = 9;
// Upper bound on one pacing sleep slice so a cancel request interrupts the
// wait quickly instead of after the full minimum interval.
const PACING_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct OscSendResult {
    pub(crate) target: String,
    pub(crate) byte_count: usize,
    pub(crate) rendered_text: String,
    pub(crate) clipped: bool,
}

pub(crate) struct ChatboxOscSender {
    socket: UdpSocket,
    target: String,
    min_interval: Duration,
    last_send: Option<Instant>,
}

impl ChatboxOscSender {
    pub(crate) fn new(config: &OscConfig) -> AppResult<Self> {
        let socket =
            UdpSocket::bind("0.0.0.0:0").map_err(|error| AppError::osc_bind(error.to_string()))?;

        Ok(Self {
            socket,
            target: format!("{}:{}", config.host, config.port),
            min_interval: Duration::from_millis(config.min_interval_ms),
            last_send: None,
        })
    }

    /// Shapes `text` for the Chatbox layout and sends it immediately.
    pub(crate) fn send(&self, text: &str) -> AppResult<OscSendResult> {
        let shaped = shape_chatbox_text(text);
        let mut result = self.send_rendered(&shaped.text)?;

        result.clipped = shaped.clipped;
        result.rendered_text = shaped.text;

        Ok(result)
    }

    /// Like [`Self::send`], but first waits out the configured minimum
    /// interval since the previous paced send. The wait polls `cancel`, so a
    /// runtime stop aborts the send instead of flushing one more Chatbox
    /// message after the user asked to stop; a cancelled call returns
    /// `Ok(None)` and leaves the pacing state untouched.
    pub(crate) fn send_paced(
        &mut self,
        text: &str,
        cancel: &AtomicBool,
    ) -> AppResult<Option<OscSendResult>> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }

            let Some(remaining) = self.remaining_pacing_wait() else {
                break;
            };

            thread::sleep(remaining.min(PACING_POLL_INTERVAL));
        }

        let result = self.send(text)?;
        self.last_send = Some(Instant::now());

        Ok(Some(result))
    }

    fn remaining_pacing_wait(&self) -> Option<Duration> {
        let elapsed = self.last_send?.elapsed();

        if elapsed >= self.min_interval {
            return None;
        }

        Some(self.min_interval - elapsed)
    }

    fn send_rendered(&self, text: &str) -> AppResult<OscSendResult> {
        let packet = chatbox_input_packet(text);
        let packet_bytes =
            encoder::encode(&packet).map_err(|error| AppError::osc_encode(error.to_string()))?;
        let sent = self
            .socket
            .send_to(&packet_bytes, &self.target)
            .map_err(|error| AppError::osc_send(&self.target, error.to_string()))?;

        if sent != packet_bytes.len() {
            return Err(AppError::osc_send_incomplete(
                &self.target,
                packet_bytes.len(),
                sent,
            ));
        }

        Ok(OscSendResult {
            target: self.target.clone(),
            byte_count: sent,
            rendered_text: text.to_string(),
            clipped: false,
        })
    }
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

    fn local_test_config(port: u16, min_interval_ms: u64) -> OscConfig {
        OscConfig {
            host: "127.0.0.1".to_string(),
            port,
            enabled: true,
            min_interval_ms,
        }
    }

    #[test]
    fn paced_send_is_cancelled_instead_of_waiting_out_the_interval() -> AppResult<()> {
        let mut sender = ChatboxOscSender::new(&local_test_config(9000, 60_000))?;
        // A fresh last_send forces the full one-minute pacing wait; the test
        // only finishes quickly if cancellation beats that wait.
        sender.last_send = Some(Instant::now());
        let last_send_before = sender.last_send;
        let cancel = AtomicBool::new(true);

        let result = sender.send_paced("cancelled", &cancel)?;

        assert!(result.is_none());
        assert_eq!(sender.last_send, last_send_before);

        Ok(())
    }

    #[test]
    fn paced_send_sends_and_tracks_pacing_when_not_cancelled() -> AppResult<()> {
        let receiver = UdpSocket::bind("127.0.0.1:0")
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let port = receiver
            .local_addr()
            .map_err(|error| AppError::osc_bind(error.to_string()))?
            .port();
        let mut sender = ChatboxOscSender::new(&local_test_config(port, 500))?;
        let cancel = AtomicBool::new(false);

        let result = sender
            .send_paced(OSC_TEST_MESSAGE, &cancel)?
            .ok_or_else(|| AppError::osc_send("test", "send was skipped".to_string()))?;

        assert!(result.byte_count > 0);
        assert!(!result.clipped);
        assert!(sender.last_send.is_some());

        Ok(())
    }
}
