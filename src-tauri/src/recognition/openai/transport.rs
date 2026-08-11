//! Production WebSocket transport for OpenAI Realtime transcription.
//!
//! The protocol state machine depends only on `RealtimeTransport`; this file
//! contains the replaceable network dependency, system-proxy tunnel, and
//! API-key handshake. Tests never contact an external network.

use super::OpenAiTranscriptionModel;
use super::attempt::RecognitionAttempt;
use super::realtime::{
    OpenAiRealtimeAttempt, OpenAiRealtimeAttemptContext, ProviderError, RealtimeTransport,
    openai_provider_failure,
};
use crate::error::{AppError, AppResult, ProviderFailureClass};
use crate::host_resolver::HostResolver;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::io::{self, ErrorKind};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::error::ProtocolError;
use tungstenite::handshake::client::Request;
use tungstenite::handshake::{HandshakeError, MidHandshake, client::ClientHandshake};
use tungstenite::protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{
    ClientRequestBuilder, Connector, Error as WebSocketError, Message, WebSocket,
    client_tls_with_config,
};

mod system_proxy;
mod tls_pump;

use tls_pump::{OpenAiTlsPump, split_established_tls};

const HANDSHAKE_IO_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const OPENAI_REALTIME_TRANSCRIPTION_WEBSOCKET_URL: &str =
    "wss://api.openai.com/v1/realtime?intent=transcription";
const ATTEMPT_READY_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPT_READY_POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 1_048_576;
const MAX_WEBSOCKET_WRITE_BUFFER_BYTES: usize = 2_097_152;
const MAX_WEBSOCKET_CONTROL_FRAMES_PER_POLL: usize = 8;

type OpenAiSocket = WebSocket<MaybeTlsStream<TcpStream>>;
type OpenAiHandshakeError = HandshakeError<ClientHandshake<MaybeTlsStream<TcpStream>>>;
type OpenAiMidHandshake = MidHandshake<ClientHandshake<MaybeTlsStream<TcpStream>>>;

pub(crate) struct OpenAiWebSocketTransport {
    socket: OpenAiSocket,
    tls_pump: Option<OpenAiTlsPump>,
    state: OpenAiWebSocketState,
}

enum OpenAiWebSocketState {
    Open,
    PeerClosePending(AppError),
    Closed,
}

impl OpenAiWebSocketTransport {
    /// Opens an authenticated TLS connection, tunneling through the selected
    /// system HTTP proxy when required. Redirects are disabled so the
    /// Authorization header can never be forwarded to a different origin.
    pub(crate) fn connect(
        api_key: &SecretString,
        resolver: &HostResolver,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self> {
        let request = openai_websocket_request(api_key)?;
        let tcp = system_proxy::connect_with_system_proxy(&request, resolver, is_cancelled)?;
        if is_cancelled() {
            let _ = tcp.shutdown(Shutdown::Both);
            return Err(startup_cancelled_error());
        }
        let websocket_config = WebSocketConfig::default()
            .write_buffer_size(64 * 1024)
            .max_write_buffer_size(MAX_WEBSOCKET_WRITE_BUFFER_BYTES)
            .max_message_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES));
        let mut socket = open_websocket_until(
            request,
            tcp,
            websocket_config,
            Instant::now() + HANDSHAKE_IO_TIMEOUT,
            is_cancelled,
        )?;
        if is_cancelled() {
            let _ = shutdown_socket(socket.get_mut());
            return Err(startup_cancelled_error());
        }
        Self::from_established_socket(socket)
    }

    fn from_established_socket(mut socket: OpenAiSocket) -> AppResult<Self> {
        configure_established_socket(socket.get_mut())?;
        let tls_pump = split_established_tls(&mut socket)?;
        Ok(Self {
            socket,
            tls_pump,
            state: OpenAiWebSocketState::Open,
        })
    }

    fn drive_tls(&mut self) -> AppResult<()> {
        if let Some(tls_pump) = self.tls_pump.as_mut() {
            tls_pump.drive()?;
        }
        Ok(())
    }

    fn note_websocket_write(&mut self) {
        if let Some(tls_pump) = self.tls_pump.as_mut() {
            tls_pump.note_websocket_write();
        }
    }

    fn flush_outbound(&mut self, context: &'static str) -> AppResult<bool> {
        self.drive_tls()?;
        self.note_websocket_write();
        let websocket_flushed = match self.socket.flush() {
            Ok(()) => true,
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => false,
            Err(error) => return Err(map_socket_error(context, error)),
        };
        self.drive_tls()?;
        Ok(websocket_flushed
            && self
                .tls_pump
                .as_ref()
                .is_none_or(OpenAiTlsPump::outbound_idle))
    }

    fn finish_peer_close(&mut self) -> AppResult<Option<String>> {
        let OpenAiWebSocketState::PeerClosePending(error) =
            std::mem::replace(&mut self.state, OpenAiWebSocketState::Closed)
        else {
            return Err(AppError::state(
                "OpenAI Realtime transport entered an invalid Close state.",
            ));
        };
        Err(error)
    }
}

fn open_websocket_until(
    request: Request,
    tcp: TcpStream,
    websocket_config: WebSocketConfig,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<OpenAiSocket> {
    open_websocket_until_with_connector(
        request,
        tcp,
        websocket_config,
        deadline,
        is_cancelled,
        None,
    )
}

fn open_websocket_until_with_connector(
    request: Request,
    tcp: TcpStream,
    websocket_config: WebSocketConfig,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
    connector: Option<Connector>,
) -> AppResult<OpenAiSocket> {
    if is_cancelled() {
        let _ = tcp.shutdown(Shutdown::Both);
        return Err(startup_cancelled_error());
    }
    if Instant::now() >= deadline {
        let _ = tcp.shutdown(Shutdown::Both);
        return Err(handshake_timeout_error());
    }
    tcp.set_nonblocking(true).map_err(|error| {
        AppError::recognition_network_terminal(format!(
            "Failed to configure cancellable OpenAI Realtime handshake I/O: {error}"
        ))
    })?;

    let mut result = client_tls_with_config(request, tcp, Some(websocket_config), connector);
    loop {
        match result {
            Ok((socket, _response)) => {
                if is_cancelled() {
                    let _ = shutdown_socket(socket.get_ref());
                    return Err(startup_cancelled_error());
                }
                if Instant::now() >= deadline {
                    let _ = shutdown_socket(socket.get_ref());
                    return Err(handshake_timeout_error());
                }
                return Ok(socket);
            }
            Err(HandshakeError::Failure(error)) => {
                return Err(map_handshake_error(HandshakeError::Failure(error)));
            }
            Err(HandshakeError::Interrupted(handshake)) => {
                if is_cancelled() {
                    let _ = shutdown_mid_handshake(&handshake);
                    return Err(startup_cancelled_error());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    let _ = shutdown_mid_handshake(&handshake);
                    return Err(handshake_timeout_error());
                }
                std::thread::sleep(remaining.min(HANDSHAKE_CANCEL_POLL_INTERVAL));
                result = handshake.handshake();
            }
        }
    }
}

fn shutdown_mid_handshake(handshake: &OpenAiMidHandshake) -> io::Result<()> {
    shutdown_socket(handshake.get_ref().get_ref())
}

fn handshake_timeout_error() -> AppError {
    AppError::recognition_network_retryable("OpenAI Realtime TLS or WebSocket handshake timed out.")
}

/// Runtime wiring entry point: credentials and transport stay inside the
/// OpenAI Module, while callers receive the provider-neutral attempt behavior.
pub(crate) fn connect_openai_realtime_attempt(
    context: OpenAiRealtimeAttemptContext,
    model: OpenAiTranscriptionModel,
    languages: Vec<String>,
    api_key: &SecretString,
    resolver: &HostResolver,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<OpenAiRealtimeAttempt<OpenAiWebSocketTransport>> {
    let transport = OpenAiWebSocketTransport::connect(api_key, resolver, is_cancelled)?;
    let mut attempt = OpenAiRealtimeAttempt::connect(context, model, languages, transport)?;
    let deadline = Instant::now() + ATTEMPT_READY_TIMEOUT;
    while !attempt.is_ready() {
        if is_cancelled() {
            let _ = attempt.stop();
            return Err(startup_cancelled_error());
        }
        if Instant::now() >= deadline {
            let _ = attempt.stop();
            return Err(AppError::recognition_network_retryable(
                "OpenAI Realtime did not confirm the transcription session configuration within 10 seconds.",
            ));
        }
        if let Err(error) = attempt.drain_events(0) {
            let _ = attempt.stop();
            return Err(error);
        }
        std::thread::sleep(ATTEMPT_READY_POLL_INTERVAL);
    }
    Ok(attempt)
}

fn startup_cancelled_error() -> AppError {
    AppError::recognition_network_terminal(
        "OpenAI Realtime connection was cancelled during startup.",
    )
}

impl RealtimeTransport for OpenAiWebSocketTransport {
    fn send_text(&mut self, message: String) -> AppResult<()> {
        match &self.state {
            OpenAiWebSocketState::Open => {}
            OpenAiWebSocketState::PeerClosePending(_) => {
                return Err(AppError::recognition_network_retryable(
                    "OpenAI Realtime WebSocket is closing.",
                ));
            }
            OpenAiWebSocketState::Closed => {
                return Err(AppError::recognition_network_retryable(
                    "OpenAI Realtime WebSocket is already closed.",
                ));
            }
        }
        self.drive_tls()?;
        self.note_websocket_write();
        let accepted = match self.socket.send(Message::text(message)) {
            Ok(()) => true,
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                // Tungstenite retains the partially written frame in its bounded
                // write buffer. A later write, read, or flush continues that
                // frame; resending this logical event would duplicate it.
                true
            }
            Err(error) => {
                return Err(map_socket_error(
                    "Failed to send an OpenAI Realtime client event",
                    error,
                ));
            }
        };
        if accepted {
            self.drive_tls()?;
        }
        Ok(())
    }

    fn try_receive_text(&mut self) -> AppResult<Option<String>> {
        if matches!(&self.state, OpenAiWebSocketState::Closed) {
            return Ok(None);
        }

        let outbound_flushed =
            self.flush_outbound("Failed to flush pending OpenAI Realtime client events")?;
        if matches!(&self.state, OpenAiWebSocketState::PeerClosePending(_)) {
            if outbound_flushed {
                return self.finish_peer_close();
            }
            return Ok(None);
        }

        let mut control_frames = 0;
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => return Ok(Some(text.as_str().to_string())),
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                    control_frames += 1;
                    if control_frames >= MAX_WEBSOCKET_CONTROL_FRAMES_PER_POLL {
                        let _ = self.flush_outbound(
                            "Failed to flush an OpenAI Realtime control response",
                        )?;
                        return Ok(None);
                    }
                }
                Ok(Message::Close(frame)) => {
                    self.state =
                        OpenAiWebSocketState::PeerClosePending(map_close_frame(frame.as_ref()));
                    if self
                        .flush_outbound("Failed to acknowledge the OpenAI Realtime peer Close")?
                    {
                        return self.finish_peer_close();
                    }
                    return Ok(None);
                }
                Ok(Message::Binary(_)) => {
                    return Err(AppError::recognition(
                        "OpenAI Realtime returned an unexpected binary WebSocket frame.",
                    ));
                }
                Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                    self.drive_tls()?;
                    return Ok(None);
                }
                Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                    self.state = OpenAiWebSocketState::Closed;
                    return Err(AppError::recognition_network_retryable(
                        "OpenAI Realtime WebSocket connection ended.",
                    ));
                }
                Err(error) => {
                    return Err(map_socket_error(
                        "Failed to read from OpenAI Realtime",
                        error,
                    ));
                }
            }
        }
    }

    fn close(&mut self) -> AppResult<()> {
        if matches!(&self.state, OpenAiWebSocketState::Closed) {
            return Ok(());
        }
        self.state = OpenAiWebSocketState::Closed;
        let shutdown_result = if let Some(tls_pump) = self.tls_pump.as_ref() {
            let result = tls_pump.shutdown();
            let _ = shutdown_socket(self.socket.get_mut());
            result
        } else {
            shutdown_socket(self.socket.get_mut())
        };
        shutdown_result.map_err(|error| {
            AppError::recognition_network_terminal(format!(
                "Failed to shut down the OpenAI Realtime socket: {error}"
            ))
        })
    }
}

fn map_close_frame(frame: Option<&CloseFrame>) -> AppError {
    let Some(frame) = frame else {
        return AppError::recognition_network_retryable(
            "OpenAI closed the Realtime WebSocket without a status code.",
        );
    };

    let code = frame.code;
    if matches!(
        code,
        CloseCode::Normal
            | CloseCode::Away
            | CloseCode::Status
            | CloseCode::Abnormal
            | CloseCode::Error
            | CloseCode::Restart
            | CloseCode::Again
    ) {
        AppError::recognition_network_retryable(format!(
            "OpenAI closed the Realtime WebSocket with retryable status code {}.",
            u16::from(code)
        ))
    } else {
        AppError::recognition(format!(
            "OpenAI closed the Realtime WebSocket with non-retryable status code {}.",
            u16::from(code)
        ))
    }
}

fn openai_websocket_request(api_key: &SecretString) -> AppResult<Request> {
    if api_key.expose_secret().trim().is_empty() {
        return Err(AppError::secret("OpenAI API key cannot be empty."));
    }
    let uri = OPENAI_REALTIME_TRANSCRIPTION_WEBSOCKET_URL
        .parse()
        .map_err(|error| {
            AppError::recognition(format!(
                "Failed to build the OpenAI Realtime WebSocket URI: {error}"
            ))
        })?;
    ClientRequestBuilder::new(uri)
        .with_header(
            "Authorization",
            format!("Bearer {}", api_key.expose_secret()),
        )
        .into_client_request()
        .map_err(|error| {
            AppError::recognition(format!(
                "Failed to build the OpenAI Realtime WebSocket request: {error}"
            ))
        })
}

fn configure_established_socket(stream: &mut MaybeTlsStream<TcpStream>) -> AppResult<()> {
    let result = match stream {
        MaybeTlsStream::Plain(tcp) => configure_established_tcp(tcp),
        MaybeTlsStream::Rustls(tls) => configure_established_tcp(&tls.sock),
        _ => Err(io::Error::other(
            "Unsupported TLS stream for OpenAI Realtime.",
        )),
    };
    result.map_err(|error| {
        AppError::recognition_network_terminal(format!(
            "Failed to configure the OpenAI Realtime socket: {error}"
        ))
    })
}

fn configure_established_tcp(tcp: &TcpStream) -> io::Result<()> {
    tcp.set_read_timeout(None)?;
    tcp.set_write_timeout(None)?;
    tcp.set_nonblocking(true)
}

fn shutdown_socket(stream: &MaybeTlsStream<TcpStream>) -> io::Result<()> {
    match stream {
        MaybeTlsStream::Plain(tcp) => tcp.shutdown(Shutdown::Both),
        MaybeTlsStream::Rustls(tls) => tls.sock.shutdown(Shutdown::Both),
        _ => Err(io::Error::other(
            "Unsupported TLS stream for OpenAI Realtime.",
        )),
    }
}

fn map_handshake_error(error: OpenAiHandshakeError) -> AppError {
    match error {
        HandshakeError::Failure(WebSocketError::Http(response)) => {
            map_handshake_http_status(response.status().as_u16(), response.body().as_deref())
        }
        HandshakeError::Failure(error) => {
            map_socket_error("Failed to connect to OpenAI Realtime", error)
        }
        HandshakeError::Interrupted(_) => handshake_timeout_error(),
    }
}

#[derive(Deserialize)]
struct HandshakeProviderErrorEnvelope {
    error: ProviderError,
}

fn map_handshake_http_status(status: u16, body: Option<&[u8]>) -> AppError {
    match status {
        401 | 403 => AppError::secret(format!(
            "OpenAI Realtime rejected the API key or project access (HTTP {status})."
        )),
        429 => {
            let class = body
                .and_then(|body| {
                    serde_json::from_slice::<HandshakeProviderErrorEnvelope>(body).ok()
                })
                .map(|envelope| envelope.error.classification())
                .filter(|class| *class != ProviderFailureClass::Unknown)
                .unwrap_or(ProviderFailureClass::RateLimited);
            openai_provider_failure(class)
        }
        500..=599 => AppError::recognition_provider(
            ProviderFailureClass::ServiceUnavailable,
            "OpenAI Realtime is temporarily unavailable.",
        ),
        _ => AppError::recognition(format!(
            "Failed to connect to OpenAI Realtime: HTTP {status}."
        )),
    }
}

fn map_socket_error(context: &str, error: WebSocketError) -> AppError {
    match error {
        WebSocketError::Io(io_error) if is_transient_io_error(io_error.kind()) => {
            AppError::recognition_network_retryable(format!("{context}: {io_error}"))
        }
        WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed => {
            AppError::recognition_network_retryable(format!("{context}: {error}"))
        }
        WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            AppError::recognition_network_retryable(format!("{context}: {error}"))
        }
        WebSocketError::WriteBufferFull(_) => AppError::recognition_backpressure(
            "The OpenAI Realtime WebSocket write buffer filled; the recognition attempt stopped instead of dropping or duplicating client events.",
        ),
        WebSocketError::Io(_) | WebSocketError::Tls(_) | WebSocketError::Url(_) => {
            AppError::recognition_network_terminal(format!("{context}: {error}"))
        }
        other => AppError::recognition(format!("{context}: {other}")),
    }
}

fn is_transient_io_error(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NetworkUnreachable
            | ErrorKind::HostUnreachable
            | ErrorKind::NotConnected
            | ErrorKind::BrokenPipe
            | ErrorKind::TimedOut
            | ErrorKind::UnexpectedEof
            | ErrorKind::Interrupted
            | ErrorKind::WouldBlock
    )
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
