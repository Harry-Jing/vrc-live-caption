//! Production WebSocket transport for OpenAI Realtime transcription.
//!
//! The protocol state machine depends only on `RealtimeTransport`; this file
//! contains the replaceable network dependency, system-proxy tunnel, and
//! API-key handshake. Tests never contact an external network.

use crate::config::OpenAiTranscriptionModel;
use crate::error::{AppError, AppResult, ProviderFailureClass};
use crate::host_resolver::HostResolver;
use crate::openai_realtime::{
    OpenAiRealtimeSession, OpenAiRealtimeSessionContext, ProviderError, RealtimeTransport,
    openai_provider_failure,
};
use crate::recognition::RecognitionSession;
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
    ClientRequestBuilder, Error as WebSocketError, Message, WebSocket, client_tls_with_config,
};

mod system_proxy;

const HANDSHAKE_IO_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const OPENAI_REALTIME_TRANSCRIPTION_WEBSOCKET_URL: &str =
    "wss://api.openai.com/v1/realtime?intent=transcription";
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const SOCKET_READ_POLL_TIMEOUT: Duration = Duration::from_millis(2);
const SESSION_READY_TIMEOUT: Duration = Duration::from_secs(10);
const SESSION_READY_POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 1_048_576;
const MAX_WEBSOCKET_WRITE_BUFFER_BYTES: usize = 2_097_152;

type OpenAiSocket = WebSocket<MaybeTlsStream<TcpStream>>;
type OpenAiHandshakeError = HandshakeError<ClientHandshake<MaybeTlsStream<TcpStream>>>;
type OpenAiMidHandshake = MidHandshake<ClientHandshake<MaybeTlsStream<TcpStream>>>;

pub(crate) struct OpenAiWebSocketTransport {
    socket: OpenAiSocket,
    closed: bool,
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
        configure_read_poll_timeout(socket.get_mut())?;
        Ok(Self {
            socket,
            closed: false,
        })
    }
}

fn open_websocket_until(
    request: Request,
    tcp: TcpStream,
    websocket_config: WebSocketConfig,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
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
        AppError::stt_network(format!(
            "Failed to configure cancellable OpenAI Realtime handshake I/O: {error}"
        ))
    })?;

    let mut result = client_tls_with_config(request, tcp, Some(websocket_config), None);
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
    AppError::stt_network_retryable("OpenAI Realtime TLS or WebSocket handshake timed out.")
}

/// Runtime wiring entry point: credentials and transport stay inside the
/// OpenAI Module, while callers receive the provider-neutral session behavior.
pub(crate) fn connect_openai_realtime_session(
    context: OpenAiRealtimeSessionContext,
    model: OpenAiTranscriptionModel,
    languages: Vec<String>,
    api_key: &SecretString,
    resolver: &HostResolver,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<OpenAiRealtimeSession<OpenAiWebSocketTransport>> {
    let transport = OpenAiWebSocketTransport::connect(api_key, resolver, is_cancelled)?;
    let mut session = OpenAiRealtimeSession::connect(context, model, languages, transport)?;
    let deadline = Instant::now() + SESSION_READY_TIMEOUT;
    while !session.is_ready() {
        if is_cancelled() {
            let _ = session.stop();
            return Err(startup_cancelled_error());
        }
        if Instant::now() >= deadline {
            let _ = session.stop();
            return Err(AppError::stt_network_retryable(
                "OpenAI Realtime did not confirm the transcription session configuration within 10 seconds.",
            ));
        }
        if let Err(error) = session.drain_events(0) {
            let _ = session.stop();
            return Err(error);
        }
        std::thread::sleep(SESSION_READY_POLL_INTERVAL);
    }
    Ok(session)
}

fn startup_cancelled_error() -> AppError {
    AppError::stt_network("OpenAI Realtime connection was cancelled during startup.")
}

impl RealtimeTransport for OpenAiWebSocketTransport {
    fn send_text(&mut self, message: String) -> AppResult<()> {
        if self.closed {
            return Err(AppError::stt_network_retryable(
                "OpenAI Realtime WebSocket is already closed.",
            ));
        }
        self.socket.send(Message::text(message)).map_err(|error| {
            map_socket_error("Failed to send an OpenAI Realtime client event", error)
        })
    }

    fn try_receive_text(&mut self) -> AppResult<Option<String>> {
        if self.closed {
            return Ok(None);
        }
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => return Ok(Some(text.as_str().to_string())),
                Ok(Message::Ping(_) | Message::Pong(_)) => continue,
                Ok(Message::Close(frame)) => {
                    self.closed = true;
                    return Err(map_close_frame(frame.as_ref()));
                }
                Ok(Message::Binary(_)) => {
                    return Err(AppError::stt(
                        "OpenAI Realtime returned an unexpected binary WebSocket frame.",
                    ));
                }
                Ok(Message::Frame(_)) => continue,
                Err(WebSocketError::Io(error))
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return Ok(None);
                }
                Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                    self.closed = true;
                    return Err(AppError::stt_network_retryable(
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
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        shutdown_socket(self.socket.get_mut()).map_err(|error| {
            AppError::stt_network(format!(
                "Failed to shut down the OpenAI Realtime socket: {error}"
            ))
        })
    }
}

fn map_close_frame(frame: Option<&CloseFrame>) -> AppError {
    let Some(frame) = frame else {
        return AppError::stt_network_retryable(
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
        AppError::stt_network_retryable(format!(
            "OpenAI closed the Realtime WebSocket with retryable status code {}.",
            u16::from(code)
        ))
    } else {
        AppError::stt(format!(
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
            AppError::stt(format!(
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
            AppError::stt(format!(
                "Failed to build the OpenAI Realtime WebSocket request: {error}"
            ))
        })
}

fn configure_read_poll_timeout(stream: &mut MaybeTlsStream<TcpStream>) -> AppResult<()> {
    let result = match stream {
        MaybeTlsStream::Plain(tcp) => configure_established_tcp(tcp),
        MaybeTlsStream::Rustls(tls) => configure_established_tcp(&tls.sock),
        _ => Err(io::Error::other(
            "Unsupported TLS stream for OpenAI Realtime.",
        )),
    };
    result.map_err(|error| {
        AppError::stt_network(format!(
            "Failed to configure the OpenAI Realtime socket: {error}"
        ))
    })
}

fn configure_established_tcp(tcp: &TcpStream) -> io::Result<()> {
    tcp.set_nonblocking(false)?;
    tcp.set_read_timeout(Some(SOCKET_READ_POLL_TIMEOUT))?;
    tcp.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))
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
        500..=599 => AppError::stt_provider(
            ProviderFailureClass::ServiceUnavailable,
            "OpenAI Realtime is temporarily unavailable.",
        ),
        _ => AppError::stt(format!(
            "Failed to connect to OpenAI Realtime: HTTP {status}."
        )),
    }
}

fn map_socket_error(context: &str, error: WebSocketError) -> AppError {
    match error {
        WebSocketError::Io(io_error) if is_transient_io_error(io_error.kind()) => {
            AppError::stt_network_retryable(format!("{context}: {io_error}"))
        }
        WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed => {
            AppError::stt_network_retryable(format!("{context}: {error}"))
        }
        WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            AppError::stt_network_retryable(format!("{context}: {error}"))
        }
        WebSocketError::Io(_) | WebSocketError::Tls(_) | WebSocketError::Url(_) => {
            AppError::stt_network(format!("{context}: {error}"))
        }
        other => AppError::stt(format!("{context}: {other}")),
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
#[path = "openai_realtime_transport_tests.rs"]
mod tests;
