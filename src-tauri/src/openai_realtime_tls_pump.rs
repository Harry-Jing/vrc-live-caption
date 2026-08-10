//! Established-connection TLS I/O pump for OpenAI Realtime.

use super::{OpenAiSocket, map_socket_error};
use crate::error::{AppError, AppResult};
use rustls::ClientConnection;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use tungstenite::Error as WebSocketError;
use tungstenite::stream::MaybeTlsStream;

const TLS_PUMP_CHUNK_BYTES: usize = 16 * 1024;
const TLS_PUMP_MAX_BYTES_PER_DIRECTION: usize = 128 * 1024;

pub(super) struct OpenAiTlsPump {
    connection: ClientConnection,
    network: TcpStream,
    websocket_io: TcpStream,
    pending_outbound_plaintext: PendingBytes,
    pending_inbound_plaintext: PendingBytes,
    websocket_outbound_drained: bool,
    network_input_closed: bool,
    websocket_input_shutdown: bool,
}

#[derive(Default)]
struct PendingBytes {
    bytes: Vec<u8>,
    offset: usize,
}

impl PendingBytes {
    fn is_empty(&self) -> bool {
        self.offset >= self.bytes.len()
    }

    fn remaining(&self) -> &[u8] {
        self.bytes.get(self.offset..).unwrap_or_default()
    }

    fn set(&mut self, bytes: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
        self.offset = 0;
    }

    fn consume(&mut self, bytes: usize) {
        self.offset = self.offset.saturating_add(bytes).min(self.bytes.len());
        if self.is_empty() {
            self.bytes.clear();
            self.offset = 0;
        }
    }
}

impl OpenAiTlsPump {
    fn new(connection: ClientConnection, network: TcpStream, websocket_io: TcpStream) -> Self {
        Self {
            connection,
            network,
            websocket_io,
            pending_outbound_plaintext: PendingBytes::default(),
            pending_inbound_plaintext: PendingBytes::default(),
            websocket_outbound_drained: true,
            network_input_closed: false,
            websocket_input_shutdown: false,
        }
    }

    pub(super) fn note_websocket_write(&mut self) {
        self.websocket_outbound_drained = false;
    }

    pub(super) fn outbound_idle(&self) -> bool {
        self.websocket_outbound_drained
            && self.pending_outbound_plaintext.is_empty()
            && !self.connection.wants_write()
    }

    pub(super) fn drive(&mut self) -> AppResult<()> {
        self.drive_outbound()?;
        self.drive_inbound()?;
        // Processing inbound TLS can queue protocol responses. Give those a
        // fair write opportunity without coupling inbound progress to it.
        self.drive_outbound()
    }

    fn drive_outbound(&mut self) -> AppResult<()> {
        let mut work_bytes = 0;
        let mut buffer = [0_u8; TLS_PUMP_CHUNK_BYTES];

        self.write_tls_records(&mut work_bytes)?;
        while work_bytes < TLS_PUMP_MAX_BYTES_PER_DIRECTION {
            if !self.pending_outbound_plaintext.is_empty() {
                let write_result = {
                    let pending = self.pending_outbound_plaintext.remaining();
                    self.connection.writer().write(pending)
                };
                match write_result {
                    Ok(0) => break,
                    Ok(written) => {
                        self.pending_outbound_plaintext.consume(written);
                        work_bytes = work_bytes.saturating_add(written);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => {
                        return Err(map_socket_error(
                            "Failed to buffer outbound OpenAI Realtime TLS plaintext",
                            WebSocketError::Io(error),
                        ));
                    }
                }
                self.write_tls_records(&mut work_bytes)?;
                if !self.pending_outbound_plaintext.is_empty() {
                    break;
                }
            }

            let remaining_budget = TLS_PUMP_MAX_BYTES_PER_DIRECTION.saturating_sub(work_bytes);
            if remaining_budget == 0 {
                self.websocket_outbound_drained = false;
                break;
            }
            let read_limit = remaining_budget.min(buffer.len());
            match self.websocket_io.read(&mut buffer[..read_limit]) {
                Ok(0) => {
                    self.websocket_outbound_drained = true;
                    break;
                }
                Ok(read) => {
                    self.websocket_outbound_drained = false;
                    work_bytes = work_bytes.saturating_add(read);
                    let written = match self.connection.writer().write(&buffer[..read]) {
                        Ok(written) => written,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => 0,
                        Err(error) if error.kind() == ErrorKind::Interrupted => 0,
                        Err(error) => {
                            return Err(map_socket_error(
                                "Failed to buffer outbound OpenAI Realtime TLS plaintext",
                                WebSocketError::Io(error),
                            ));
                        }
                    };
                    if written < read {
                        self.pending_outbound_plaintext.set(&buffer[written..read]);
                    }
                    self.write_tls_records(&mut work_bytes)?;
                    if !self.pending_outbound_plaintext.is_empty() {
                        break;
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.websocket_outbound_drained = true;
                    break;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(AppError::stt_network_terminal(format!(
                        "Failed to read the OpenAI Realtime WebSocket owner channel: {error}"
                    )));
                }
            }
        }
        self.write_tls_records(&mut work_bytes)
    }

    fn write_tls_records(&mut self, work_bytes: &mut usize) -> AppResult<()> {
        while self.connection.wants_write() && *work_bytes < TLS_PUMP_MAX_BYTES_PER_DIRECTION {
            match self.connection.write_tls(&mut self.network) {
                Ok(0) => break,
                Ok(written) => *work_bytes = work_bytes.saturating_add(written),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(map_socket_error(
                        "Failed to write OpenAI Realtime TLS records",
                        WebSocketError::Io(error),
                    ));
                }
            }
        }
        Ok(())
    }

    fn drive_inbound(&mut self) -> AppResult<()> {
        let mut work_bytes = 0;
        if !self.flush_inbound_plaintext(&mut work_bytes)? {
            return Ok(());
        }
        if !self.drain_decrypted_plaintext(&mut work_bytes)? {
            return Ok(());
        }

        while !self.network_input_closed && work_bytes < TLS_PUMP_MAX_BYTES_PER_DIRECTION {
            match self.connection.read_tls(&mut self.network) {
                Ok(0) => {
                    self.network_input_closed = true;
                    break;
                }
                Ok(read) => {
                    work_bytes = work_bytes.saturating_add(read);
                    let io_state = self.connection.process_new_packets().map_err(|error| {
                        map_socket_error(
                            "Failed to process inbound OpenAI Realtime TLS records",
                            WebSocketError::Tls(error.into()),
                        )
                    })?;
                    if io_state.peer_has_closed() {
                        self.network_input_closed = true;
                    }
                    if !self.drain_decrypted_plaintext(&mut work_bytes)? {
                        return Ok(());
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                // rustls uses `Other` to signal that its bounded plaintext
                // buffer must be drained before more TLS input is accepted.
                Err(error) if error.kind() == ErrorKind::Other => break,
                Err(error) => {
                    return Err(map_socket_error(
                        "Failed to read OpenAI Realtime TLS records",
                        WebSocketError::Io(error),
                    ));
                }
            }
        }

        if self.network_input_closed
            && self.pending_inbound_plaintext.is_empty()
            && !self.websocket_input_shutdown
        {
            match self.websocket_io.shutdown(Shutdown::Write) {
                Ok(()) => self.websocket_input_shutdown = true,
                Err(error) if error.kind() == ErrorKind::NotConnected => {
                    self.websocket_input_shutdown = true;
                }
                Err(error) => {
                    return Err(AppError::stt_network_terminal(format!(
                        "Failed to close the OpenAI Realtime WebSocket owner input: {error}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn drain_decrypted_plaintext(&mut self, work_bytes: &mut usize) -> AppResult<bool> {
        let mut buffer = [0_u8; TLS_PUMP_CHUNK_BYTES];
        while *work_bytes < TLS_PUMP_MAX_BYTES_PER_DIRECTION {
            let remaining_budget = TLS_PUMP_MAX_BYTES_PER_DIRECTION.saturating_sub(*work_bytes);
            let read_limit = remaining_budget.min(buffer.len());
            match self.connection.reader().read(&mut buffer[..read_limit]) {
                Ok(0) => {
                    self.network_input_closed = true;
                    return Ok(true);
                }
                Ok(read) => {
                    *work_bytes = work_bytes.saturating_add(read);
                    let written = match self.websocket_io.write(&buffer[..read]) {
                        Ok(written) => written,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => 0,
                        Err(error) if error.kind() == ErrorKind::Interrupted => 0,
                        Err(error) => {
                            return Err(AppError::stt_network_terminal(format!(
                                "Failed to feed the OpenAI Realtime WebSocket owner: {error}"
                            )));
                        }
                    };
                    if written < read {
                        self.pending_inbound_plaintext.set(&buffer[written..read]);
                        return Ok(false);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(true),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(map_socket_error(
                        "Failed to read decrypted OpenAI Realtime TLS data",
                        WebSocketError::Io(error),
                    ));
                }
            }
        }
        Ok(false)
    }

    fn flush_inbound_plaintext(&mut self, work_bytes: &mut usize) -> AppResult<bool> {
        while !self.pending_inbound_plaintext.is_empty()
            && *work_bytes < TLS_PUMP_MAX_BYTES_PER_DIRECTION
        {
            let write_result = {
                let pending = self.pending_inbound_plaintext.remaining();
                self.websocket_io.write(pending)
            };
            match write_result {
                Ok(0) => {
                    return Err(AppError::stt_network_terminal(
                        "OpenAI Realtime WebSocket owner input closed unexpectedly.",
                    ));
                }
                Ok(written) => {
                    self.pending_inbound_plaintext.consume(written);
                    *work_bytes = work_bytes.saturating_add(written);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(AppError::stt_network_terminal(format!(
                        "Failed to feed pending OpenAI Realtime WebSocket bytes: {error}"
                    )));
                }
            }
        }
        Ok(self.pending_inbound_plaintext.is_empty())
    }

    pub(super) fn shutdown(&self) -> io::Result<()> {
        let network_result = self.network.shutdown(Shutdown::Both);
        let _ = self.websocket_io.shutdown(Shutdown::Both);
        network_result
    }
}

/// Separates rustls record I/O from tungstenite after the authenticated
/// handshake. Tungstenite keeps its parser and any bytes read past the HTTP
/// upgrade, while this owner can always advance TLS reads even when writes to
/// the remote peer are backpressured.
pub(super) fn split_established_tls(socket: &mut OpenAiSocket) -> AppResult<Option<OpenAiTlsPump>> {
    if !matches!(socket.get_ref(), MaybeTlsStream::Rustls(_)) {
        return Ok(None);
    }

    let (websocket_stream, pump_stream) = nonblocking_loopback_pair()?;
    let established_tls =
        std::mem::replace(socket.get_mut(), MaybeTlsStream::Plain(websocket_stream));
    let MaybeTlsStream::Rustls(established_tls) = established_tls else {
        return Err(AppError::state(
            "OpenAI Realtime TLS transport changed while establishing its I/O owner.",
        ));
    };
    let (connection, network) = established_tls.into_parts();
    Ok(Some(OpenAiTlsPump::new(connection, network, pump_stream)))
}

fn nonblocking_loopback_pair() -> AppResult<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
        AppError::stt_network_terminal(format!(
            "Failed to create the OpenAI Realtime TLS owner channel: {error}"
        ))
    })?;
    let address = listener.local_addr().map_err(|error| {
        AppError::stt_network_terminal(format!(
            "Failed to address the OpenAI Realtime TLS owner channel: {error}"
        ))
    })?;
    let websocket_stream = TcpStream::connect(address).map_err(|error| {
        AppError::stt_network_terminal(format!(
            "Failed to connect the OpenAI Realtime TLS owner channel: {error}"
        ))
    })?;
    let expected_peer = websocket_stream.local_addr().map_err(|error| {
        AppError::stt_network_terminal(format!(
            "Failed to identify the OpenAI Realtime TLS owner channel: {error}"
        ))
    })?;
    let (pump_stream, peer) = listener.accept().map_err(|error| {
        AppError::stt_network_terminal(format!(
            "Failed to accept the OpenAI Realtime TLS owner channel: {error}"
        ))
    })?;
    if peer != expected_peer {
        return Err(AppError::stt_network_terminal(
            "OpenAI Realtime TLS owner channel accepted an unexpected local peer.",
        ));
    }
    for stream in [&websocket_stream, &pump_stream] {
        stream
            .set_nodelay(true)
            .and_then(|()| stream.set_nonblocking(true))
            .map_err(|error| {
                AppError::stt_network_terminal(format!(
                    "Failed to configure the OpenAI Realtime TLS owner channel: {error}"
                ))
            })?;
    }
    Ok((websocket_stream, pump_stream))
}
