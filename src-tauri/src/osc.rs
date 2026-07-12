//! VRChat Chatbox output over OSC (UDP).
//!
//! Text is shaped to VRChat's fixed Chatbox constraints before sending. Input
//! is hard-clipped to 144 UTF-16 code units at a grapheme-cluster boundary,
//! then a layout simulation finds the source-text prefix visible within nine
//! wrapped lines. VRChat receives that prefix without artificial line breaks
//! and performs the actual wrapping. The width estimates approximate the
//! measured TMP contract in `docs/research/vrchat-chatbox-reference.md`.
//!
//! `ChatboxOscSender` owns one UDP socket for its lifetime and paces repeated
//! text sends with a minimum interval so VRChat's Chatbox rate limit does not
//! drop updates. A cloneable activity handle shares that socket, aggregates
//! active utterance ids, and sends typing transitions immediately without
//! reading or changing the text pacing state.

use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use std::collections::HashSet;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const OSC_CHATBOX_INPUT_ADDRESS: &str = "/chatbox/input";
pub(crate) const OSC_CHATBOX_TYPING_ADDRESS: &str = "/chatbox/typing";
pub(crate) const OSC_TEST_MESSAGE: &str = "VRC Live Caption OSC test.";
// ChatText rectangle minus margins; see the module docs for the source.
const CHATBOX_WIDTH_PX: f32 = 280.0;
const CHATBOX_MAX_LINES: usize = 9;
const CHATBOX_MAX_UTF16_UNITS: usize = 144;
// Upper bound on one pacing sleep slice so a cancel request interrupts the
// wait quickly instead of after the full minimum interval.
const PACING_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct OscSendResult {
    pub(crate) target: String,
    pub(crate) byte_count: usize,
    pub(crate) rendered_text: String,
    pub(crate) clipped: bool,
}

pub(crate) struct ChatboxFinalSendAttempt {
    pub(crate) text: AppResult<Option<OscSendResult>>,
    pub(crate) typing: AppResult<()>,
}

pub(crate) struct ChatboxOscSender {
    transport: Arc<dyn OscTransport>,
    activity: ChatboxActivityHandle,
    min_interval: Duration,
    last_send: Option<Instant>,
}

trait OscTransport: Send + Sync {
    fn send_packet(&self, packet: &OscPacket) -> AppResult<usize>;
    fn target(&self) -> &str;
}

struct UdpOscTransport {
    socket: UdpSocket,
    target: String,
    target_address: SocketAddr,
}

#[derive(Clone)]
pub(crate) struct ChatboxActivityHandle {
    transport: Arc<dyn OscTransport>,
    state: Arc<Mutex<ChatboxActivityState>>,
}

struct ChatboxActivityState {
    active_utterances: HashSet<String>,
    lifecycle: ChatboxActivityLifecycle,
    stop_off_attempted: bool,
    typing_on_sent: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChatboxActivityLifecycle {
    Running,
    StopRequested,
    ErrorFinished,
}

impl ChatboxOscSender {
    pub(crate) fn new(config: &OscConfig) -> AppResult<Self> {
        let socket =
            UdpSocket::bind("0.0.0.0:0").map_err(|error| AppError::osc_bind(error.to_string()))?;
        socket
            .set_nonblocking(true)
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let target = format!("{}:{}", config.host, config.port);
        let target_address = target
            .to_socket_addrs()
            .map_err(|error| AppError::osc_send(&target, error.to_string()))?
            .find(|address| address.is_ipv4())
            .ok_or_else(|| {
                AppError::osc_send(&target, "No IPv4 target address resolved.".to_string())
            })?;
        let transport: Arc<dyn OscTransport> = Arc::new(UdpOscTransport {
            socket,
            target,
            target_address,
        });

        Ok(Self::with_transport(
            transport,
            Duration::from_millis(config.min_interval_ms),
        ))
    }

    fn with_transport(transport: Arc<dyn OscTransport>, min_interval: Duration) -> Self {
        let activity = ChatboxActivityHandle {
            transport: Arc::clone(&transport),
            state: Arc::new(Mutex::new(ChatboxActivityState {
                active_utterances: HashSet::new(),
                lifecycle: ChatboxActivityLifecycle::Running,
                stop_off_attempted: false,
                typing_on_sent: false,
            })),
        };

        Self {
            transport,
            activity,
            min_interval,
            last_send: None,
        }
    }

    pub(crate) fn activity_handle(&self) -> ChatboxActivityHandle {
        self.activity.clone()
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

        let result = {
            let state = self
                .activity
                .state
                .lock()
                .map_err(|_| AppError::state("Chatbox activity state lock was poisoned."))?;

            if state.lifecycle != ChatboxActivityLifecycle::Running
                || cancel.load(Ordering::Relaxed)
            {
                return Ok(None);
            }

            self.send(text)?
        };
        self.last_send = Some(Instant::now());

        Ok(Some(result))
    }

    pub(crate) fn send_final_paced(
        &mut self,
        utterance_id: &str,
        text: &str,
        cancel: &AtomicBool,
    ) -> ChatboxFinalSendAttempt {
        let text = self.send_paced(text, cancel);
        let typing = self.activity.utterance_resolved(utterance_id);

        ChatboxFinalSendAttempt { text, typing }
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
        let sent = self.transport.send_packet(&packet)?;

        Ok(OscSendResult {
            target: self.transport.target().to_string(),
            byte_count: sent,
            rendered_text: text.to_string(),
            clipped: false,
        })
    }
}

impl OscTransport for UdpOscTransport {
    fn send_packet(&self, packet: &OscPacket) -> AppResult<usize> {
        let packet_bytes =
            encoder::encode(packet).map_err(|error| AppError::osc_encode(error.to_string()))?;
        let sent = self
            .socket
            .send_to(&packet_bytes, self.target_address)
            .map_err(|error| AppError::osc_send(&self.target, error.to_string()))?;

        if sent != packet_bytes.len() {
            return Err(AppError::osc_send_incomplete(
                &self.target,
                packet_bytes.len(),
                sent,
            ));
        }

        Ok(sent)
    }

    fn target(&self) -> &str {
        &self.target
    }
}

impl ChatboxActivityHandle {
    pub(crate) fn utterance_started(&self, utterance_id: &str) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Chatbox activity state lock was poisoned."))?;

        if state.lifecycle != ChatboxActivityLifecycle::Running {
            return Ok(());
        }

        let was_empty = state.active_utterances.is_empty();

        if !state.active_utterances.insert(utterance_id.to_string()) || !was_empty {
            return Ok(());
        }

        self.transport.send_packet(&typing_indicator_packet(true))?;
        state.typing_on_sent = true;

        Ok(())
    }

    pub(crate) fn utterance_resolved(&self, utterance_id: &str) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Chatbox activity state lock was poisoned."))?;

        if state.lifecycle != ChatboxActivityLifecycle::Running
            || !state.active_utterances.remove(utterance_id)
            || !state.active_utterances.is_empty()
        {
            return Ok(());
        }

        self.transport
            .send_packet(&typing_indicator_packet(false))?;
        state.typing_on_sent = false;

        Ok(())
    }

    pub(crate) fn request_stop(&self, cancel: &AtomicBool) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Chatbox activity state lock was poisoned."))?;
        cancel.store(true, Ordering::Relaxed);
        state.lifecycle = ChatboxActivityLifecycle::StopRequested;
        state.active_utterances.clear();

        Ok(())
    }

    pub(crate) fn finish_stop(&self) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Chatbox activity state lock was poisoned."))?;

        if state.stop_off_attempted {
            return Ok(());
        }

        state.lifecycle = ChatboxActivityLifecycle::StopRequested;
        state.active_utterances.clear();
        state.stop_off_attempted = true;
        self.transport
            .send_packet(&typing_indicator_packet(false))?;
        state.typing_on_sent = false;

        Ok(())
    }

    pub(crate) fn finish_after_error(&self) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::state("Chatbox activity state lock was poisoned."))?;

        if state.lifecycle == ChatboxActivityLifecycle::ErrorFinished || state.stop_off_attempted {
            return Ok(());
        }

        let should_clear =
            state.lifecycle == ChatboxActivityLifecycle::StopRequested || state.typing_on_sent;
        state.lifecycle = ChatboxActivityLifecycle::ErrorFinished;
        state.active_utterances.clear();

        if should_clear {
            self.transport
                .send_packet(&typing_indicator_packet(false))?;
            state.stop_off_attempted = true;
            state.typing_on_sent = false;
        }

        Ok(())
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

fn typing_indicator_packet(is_typing: bool) -> OscPacket {
    OscPacket::Message(OscMessage {
        addr: OSC_CHATBOX_TYPING_ADDRESS.to_string(),
        args: vec![OscType::Bool(is_typing)],
    })
}

struct ShapedChatboxText {
    text: String,
    clipped: bool,
}

fn shape_chatbox_text(text: &str) -> ShapedChatboxText {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let (input_limited, input_clipped) = clip_to_utf16_budget(&normalized);
    let (visible, layout_clipped) = clip_to_visible_lines(input_limited);

    ShapedChatboxText {
        text: visible.to_string(),
        clipped: input_clipped || layout_clipped,
    }
}

fn clip_to_utf16_budget(text: &str) -> (&str, bool) {
    let mut used_units = 0;

    for (index, grapheme) in text.grapheme_indices(true) {
        let grapheme_units = grapheme.encode_utf16().count();

        if used_units + grapheme_units > CHATBOX_MAX_UTF16_UNITS {
            let Some(prefix) = text.get(..index) else {
                return ("", true);
            };

            return (prefix, true);
        }

        used_units += grapheme_units;
    }

    (text, false)
}

fn clip_to_visible_lines(text: &str) -> (&str, bool) {
    let mut line_count = 1;
    let mut current_width = 0.0;
    let mut line_has_content = false;

    for (index, grapheme) in text.grapheme_indices(true) {
        let width = estimate_chatbox_width_px(grapheme);

        if line_has_content && current_width + width > CHATBOX_WIDTH_PX {
            if line_count >= CHATBOX_MAX_LINES {
                let Some(prefix) = text.get(..index) else {
                    return ("", true);
                };

                return (prefix.trim_end(), true);
            }

            line_count += 1;
            current_width = 0.0;
            line_has_content = false;

            // VRChat keeps the source space in its input budget but does not
            // render it as the first character on the wrapped line.
            if grapheme.trim().is_empty() {
                continue;
            }
        }

        current_width += width;
        line_has_content = true;
    }

    (text.trim_end(), false)
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
    use rosc::decoder;
    use std::collections::VecDeque;
    use std::sync::Barrier;

    struct ScriptedOscTransport {
        failures: Mutex<VecDeque<bool>>,
        packets: Mutex<Vec<OscPacket>>,
    }

    impl OscTransport for ScriptedOscTransport {
        fn send_packet(&self, packet: &OscPacket) -> AppResult<usize> {
            self.packets
                .lock()
                .map_err(|_| AppError::state("Scripted OSC packet lock was poisoned."))?
                .push(packet.clone());
            let should_fail = self
                .failures
                .lock()
                .map_err(|_| AppError::state("Scripted OSC outcome lock was poisoned."))?
                .pop_front()
                .unwrap_or(false);

            if should_fail {
                return Err(AppError::osc_send(
                    self.target(),
                    "scripted send failure".to_string(),
                ));
            }

            encoder::encode(packet)
                .map(|bytes| bytes.len())
                .map_err(|error| AppError::osc_encode(error.to_string()))
        }

        fn target(&self) -> &str {
            "scripted:test"
        }
    }

    impl ScriptedOscTransport {
        fn packets(&self) -> AppResult<Vec<OscPacket>> {
            self.packets
                .lock()
                .map(|packets| packets.clone())
                .map_err(|_| AppError::state("Scripted OSC packet lock was poisoned."))
        }
    }

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
    fn typing_off_packet_uses_the_vrchat_boolean_contract() {
        assert_eq!(
            typing_indicator_packet(false),
            OscPacket::Message(OscMessage {
                addr: "/chatbox/typing".to_string(),
                args: vec![OscType::Bool(false)],
            })
        );
    }

    #[test]
    fn localhost_target_uses_an_ipv4_address_for_the_ipv4_socket() -> AppResult<()> {
        let receiver = UdpSocket::bind("127.0.0.1:0")
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let port = receiver
            .local_addr()
            .map_err(|error| AppError::osc_bind(error.to_string()))?
            .port();
        let mut config = local_test_config(port, 500);
        config.host = "localhost".to_string();
        let sender = ChatboxOscSender::new(&config)?;

        sender.send("test")?;

        assert_eq!(receive_osc_packet(&receiver)?, chatbox_input_packet("test"));

        Ok(())
    }

    #[test]
    fn first_active_utterance_turns_typing_indicator_on() -> AppResult<()> {
        let (sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();

        activity.utterance_started("speech-1")?;

        assert_eq!(
            receive_osc_packet(&receiver)?,
            OscPacket::Message(OscMessage {
                addr: "/chatbox/typing".to_string(),
                args: vec![OscType::Bool(true)],
            })
        );

        Ok(())
    }

    #[test]
    fn typing_indicator_stays_on_until_every_utterance_resolves() -> AppResult<()> {
        let (sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();

        activity.utterance_started("speech-1")?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(true)
        );

        activity.utterance_started("speech-1")?;
        activity.utterance_started("speech-2")?;
        assert_no_osc_packet(&receiver)?;

        activity.utterance_resolved("speech-1")?;
        assert_no_osc_packet(&receiver)?;

        activity.utterance_resolved("speech-2")?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );

        Ok(())
    }

    #[test]
    fn final_text_is_sent_before_the_last_utterance_turns_typing_off() -> AppResult<()> {
        let (mut sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.utterance_started("speech-1")?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(true)
        );

        let attempt = sender.send_final_paced("speech-1", "final text", &cancel);

        assert!(matches!(attempt.text, Ok(Some(_))));
        assert!(attempt.typing.is_ok());
        assert_eq!(
            receive_osc_packet(&receiver)?,
            chatbox_input_packet("final text")
        );
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );

        Ok(())
    }

    #[test]
    fn final_send_failure_still_attempts_to_turn_typing_off() -> AppResult<()> {
        let (mut sender, transport) = scripted_test_sender([false, true, false]);
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.utterance_started("speech-1")?;
        let attempt = sender.send_final_paced("speech-1", "final text", &cancel);

        assert!(attempt.text.is_err());
        assert!(attempt.typing.is_ok());
        assert_eq!(
            transport.packets()?,
            vec![
                typing_indicator_packet(true),
                chatbox_input_packet("final text"),
                typing_indicator_packet(false),
            ]
        );

        Ok(())
    }

    #[test]
    fn earlier_final_does_not_clear_a_later_active_utterance() -> AppResult<()> {
        let (mut sender, receiver) = local_test_sender_and_receiver(0)?;
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.utterance_started("speech-1")?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(true)
        );
        activity.utterance_started("speech-2")?;

        let first = sender.send_final_paced("speech-1", "first final", &cancel);
        assert!(matches!(first.text, Ok(Some(_))));
        assert!(first.typing.is_ok());
        assert_eq!(
            receive_osc_packet(&receiver)?,
            chatbox_input_packet("first final")
        );
        assert_no_osc_packet(&receiver)?;

        let second = sender.send_final_paced("speech-2", "second final", &cancel);
        assert!(matches!(second.text, Ok(Some(_))));
        assert!(second.typing.is_ok());
        assert_eq!(
            receive_osc_packet(&receiver)?,
            chatbox_input_packet("second final")
        );
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );

        Ok(())
    }

    #[test]
    fn stop_turns_typing_off_once_and_blocks_late_output() -> AppResult<()> {
        let (mut sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.utterance_started("speech-1")?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(true)
        );

        activity.request_stop(&cancel)?;
        assert!(cancel.load(Ordering::Relaxed));
        assert_no_osc_packet(&receiver)?;

        activity.finish_stop()?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );

        activity.request_stop(&cancel)?;
        activity.finish_stop()?;
        activity.utterance_started("speech-2")?;
        let attempt = sender.send_final_paced("speech-2", "late final", &cancel);

        assert!(matches!(attempt.text, Ok(None)));
        assert!(attempt.typing.is_ok());
        assert_no_osc_packet(&receiver)?;

        Ok(())
    }

    #[test]
    fn stop_sends_one_typing_off_even_without_an_active_utterance() -> AppResult<()> {
        let (sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.request_stop(&cancel)?;
        activity.finish_stop()?;

        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );
        activity.finish_stop()?;
        assert_no_osc_packet(&receiver)?;

        Ok(())
    }

    #[test]
    fn stop_cleanup_does_not_retry_its_failed_transition() -> AppResult<()> {
        let (sender, transport) = scripted_test_sender([false, true]);
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.utterance_started("speech-1")?;
        activity.request_stop(&cancel)?;
        assert!(activity.finish_stop().is_err());

        activity.finish_stop()?;
        assert_eq!(
            transport.packets()?,
            vec![
                typing_indicator_packet(true),
                typing_indicator_packet(false),
            ]
        );

        Ok(())
    }

    #[test]
    fn formal_stop_retries_after_failed_runtime_error_cleanup() -> AppResult<()> {
        let (sender, transport) = scripted_test_sender([false, true, false]);
        let activity = sender.activity_handle();
        let cancel = AtomicBool::new(false);

        activity.utterance_started("speech-1")?;
        assert!(activity.finish_after_error().is_err());

        activity.request_stop(&cancel)?;
        activity.finish_stop()?;
        assert_eq!(
            transport.packets()?,
            vec![
                typing_indicator_packet(true),
                typing_indicator_packet(false),
                typing_indicator_packet(false),
            ]
        );

        Ok(())
    }

    #[test]
    fn typing_transition_bypasses_an_in_progress_text_pacing_wait() -> AppResult<()> {
        let (mut sender, receiver) = local_test_sender_and_receiver(60_000)?;
        sender.last_send = Some(Instant::now());
        let activity = sender.activity_handle();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);
        let worker = thread::spawn(move || {
            worker_barrier.wait();
            sender.send_paced("delayed final", &worker_cancel)
        });

        barrier.wait();
        thread::sleep(Duration::from_millis(150));
        activity.utterance_started("speech-1")?;

        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(true)
        );

        activity.request_stop(&cancel)?;
        activity.finish_stop()?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );

        let send_result = worker
            .join()
            .map_err(|_| AppError::runtime("Paced send test thread panicked."))?;
        assert!(matches!(send_result, Ok(None)));

        Ok(())
    }

    #[test]
    fn runtime_error_clears_active_typing_and_latches_output_closed() -> AppResult<()> {
        let (sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();

        activity.utterance_started("speech-1")?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(true)
        );

        activity.finish_after_error()?;
        assert_eq!(
            receive_osc_packet(&receiver)?,
            typing_indicator_packet(false)
        );

        activity.utterance_started("speech-2")?;
        activity.finish_after_error()?;
        assert_no_osc_packet(&receiver)?;

        Ok(())
    }

    #[test]
    fn repeated_runtime_error_cleanup_without_typing_is_a_no_op() -> AppResult<()> {
        let (sender, receiver) = local_test_sender_and_receiver(500)?;
        let activity = sender.activity_handle();

        activity.finish_after_error()?;
        assert_no_osc_packet(&receiver)?;

        activity.finish_after_error()?;
        assert_no_osc_packet(&receiver)?;

        Ok(())
    }

    #[test]
    fn chatbox_send_clips_to_nine_visible_lines_without_inserting_breaks() -> AppResult<()> {
        let text = "中".repeat(144);
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&text)?;

        assert_eq!(result.rendered_text, "中".repeat(135));
        assert!(!result.rendered_text.contains('\n'));
        assert!(result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_leaves_wrapping_to_vrchat() -> AppResult<()> {
        let text = "x".repeat(40);
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&text)?;

        assert_eq!(result.rendered_text, text);
        assert!(!result.rendered_text.contains('\n'));
        assert!(!result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_hard_clips_input_to_144_utf16_code_units() -> AppResult<()> {
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&"x".repeat(145))?;

        assert_eq!(result.rendered_text, "x".repeat(144));
        assert_eq!(result.rendered_text.encode_utf16().count(), 144);
        assert!(result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_keeps_exactly_144_utf16_code_units() -> AppResult<()> {
        let text = "x".repeat(144);
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&text)?;

        assert_eq!(result.rendered_text, text);
        assert_eq!(result.rendered_text.encode_utf16().count(), 144);
        assert!(!result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_counts_non_bmp_emoji_as_two_utf16_units() -> AppResult<()> {
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&"😀".repeat(73))?;

        assert_eq!(result.rendered_text, "😀".repeat(72));
        assert_eq!(result.rendered_text.encode_utf16().count(), 144);
        assert!(result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_does_not_split_a_combining_grapheme() -> AppResult<()> {
        let grapheme = "e\u{301}";
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&grapheme.repeat(73))?;

        assert_eq!(result.rendered_text, grapheme.repeat(72));
        assert_eq!(result.rendered_text.encode_utf16().count(), 144);
        assert!(result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_does_not_split_a_zwj_emoji_sequence() -> AppResult<()> {
        let family = "👨‍👩‍👧‍👦";
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send(&family.repeat(14))?;

        assert_eq!(family.encode_utf16().count(), 11);
        assert_eq!(result.rendered_text, family.repeat(13));
        assert_eq!(result.rendered_text.encode_utf16().count(), 143);
        assert!(result.clipped);

        Ok(())
    }

    #[test]
    fn chatbox_send_normalizes_whitespace_without_reporting_clipping() -> AppResult<()> {
        let sender = ChatboxOscSender::new(&local_test_config(9000, 500))?;
        let result = sender.send("  hello\t \n world  ")?;

        assert_eq!(result.rendered_text, "hello world");
        assert!(!result.clipped);

        Ok(())
    }

    fn local_test_config(port: u16, min_interval_ms: u64) -> OscConfig {
        OscConfig {
            host: "127.0.0.1".to_string(),
            port,
            enabled: true,
            min_interval_ms,
        }
    }

    fn local_test_sender_and_receiver(
        min_interval_ms: u64,
    ) -> AppResult<(ChatboxOscSender, UdpSocket)> {
        let receiver = UdpSocket::bind("127.0.0.1:0")
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let port = receiver
            .local_addr()
            .map_err(|error| AppError::osc_bind(error.to_string()))?
            .port();
        let sender = ChatboxOscSender::new(&local_test_config(port, min_interval_ms))?;

        Ok((sender, receiver))
    }

    fn scripted_test_sender(
        failures: impl IntoIterator<Item = bool>,
    ) -> (ChatboxOscSender, Arc<ScriptedOscTransport>) {
        let transport = Arc::new(ScriptedOscTransport {
            failures: Mutex::new(failures.into_iter().collect()),
            packets: Mutex::new(Vec::new()),
        });
        let sender_transport: Arc<dyn OscTransport> = transport.clone();
        let sender = ChatboxOscSender::with_transport(sender_transport, Duration::from_millis(500));

        (sender, transport)
    }

    fn receive_osc_packet(receiver: &UdpSocket) -> AppResult<OscPacket> {
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let mut buffer = [0_u8; 1024];
        let (size, _) = receiver
            .recv_from(&mut buffer)
            .map_err(|error| AppError::osc_send("test receiver", error.to_string()))?;
        let (_, packet) = decoder::decode_udp(&buffer[..size])
            .map_err(|error| AppError::osc_encode(error.to_string()))?;

        Ok(packet)
    }

    fn assert_no_osc_packet(receiver: &UdpSocket) -> AppResult<()> {
        receiver
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|error| AppError::osc_bind(error.to_string()))?;
        let mut buffer = [0_u8; 1024];

        match receiver.recv_from(&mut buffer) {
            Ok((size, _)) => Err(AppError::runtime(format!(
                "Expected no OSC packet, but received {size} bytes."
            ))),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(AppError::osc_send("test receiver", error.to_string())),
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
