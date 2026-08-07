//! Raw VRChat Chatbox OSC transport over UDP.
//!
//! This module only encodes and attempts OSC packets. Completed layout,
//! pagination, queueing, pacing, typing lifecycle, diagnostics, and generation
//! cancellation belong to the independent Chatbox publisher. The OSC Test
//! command acquires the same process-wide pacer before calling this transport.

use crate::chatbox_publisher::{ChatboxSendReceipt, ChatboxTransport};
use crate::config::OscConfig;
use crate::error::{AppError, AppResult};
use crate::host_resolver::{HostResolutionError, HostResolver};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const OSC_CHATBOX_INPUT_ADDRESS: &str = "/chatbox/input";
pub(crate) const OSC_CHATBOX_TYPING_ADDRESS: &str = "/chatbox/typing";
pub(crate) const OSC_TEST_MESSAGE: &str = "VRC Live Caption OSC test.";
const OSC_RESOLUTION_BUDGET: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct ChatboxOscSender {
    transport: Arc<dyn OscTransport>,
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

impl ChatboxOscSender {
    pub(crate) fn new(
        config: &OscConfig,
        resolver: &HostResolver,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self> {
        Self::new_until(
            config,
            resolver,
            Instant::now() + OSC_RESOLUTION_BUDGET,
            is_cancelled,
        )
    }

    fn new_until(
        config: &OscConfig,
        resolver: &HostResolver,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self> {
        let target = format!("{}:{}", config.host, config.port);
        let target_address = resolver
            .resolve_until(&config.host, config.port, deadline, is_cancelled)
            .map_err(|error| map_resolution_error(&target, error))?
            .into_iter()
            .find(|address| address.is_ipv4())
            .ok_or_else(|| {
                AppError::osc_send(&target, "No IPv4 target address resolved.".to_string())
            })?;
        let socket =
            UdpSocket::bind("0.0.0.0:0").map_err(|error| AppError::osc_bind(error.to_string()))?;
        socket
            .set_nonblocking(true)
            .map_err(|error| AppError::osc_bind(error.to_string()))?;

        Ok(Self {
            transport: Arc::new(UdpOscTransport {
                socket,
                target,
                target_address,
            }),
        })
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn OscTransport>) -> Self {
        Self { transport }
    }

    pub(crate) fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        let byte_count = self.transport.send_packet(&chatbox_input_packet(text))?;

        Ok(ChatboxSendReceipt {
            target: self.transport.target().to_string(),
            byte_count,
        })
    }

    pub(crate) fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        self.transport
            .send_packet(&typing_indicator_packet(is_typing))?;
        Ok(())
    }
}

fn map_resolution_error(target: &str, error: HostResolutionError) -> AppError {
    let message = match error {
        HostResolutionError::Cancelled => "Hostname resolution was cancelled.".to_string(),
        HostResolutionError::DeadlineExceeded => format!(
            "Hostname resolution timed out after {} seconds.",
            OSC_RESOLUTION_BUDGET.as_secs()
        ),
        HostResolutionError::LookupFailed(detail) => {
            format!("Hostname resolution failed: {detail}")
        }
        HostResolutionError::WorkerUnavailable(detail) => {
            format!("Hostname resolver is unavailable: {detail}")
        }
        HostResolutionError::QueueFull => "Hostname resolver queue is full; try again.".to_string(),
    };
    AppError::osc_send(target, message)
}

impl ChatboxTransport for ChatboxOscSender {
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        Self::send_text(self, text)
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        Self::send_typing(self, is_typing)
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

#[cfg(test)]
#[path = "osc_tests.rs"]
mod tests;
