use mio::{Events, Interest, Poll, Token};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const REQUEST_BYTE_LIMIT: usize = 64 * 1024;
const RESPONSE_BYTE_LIMIT: usize = 128 * 1024;
const LISTENER_TOKEN: Token = Token(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FixtureStage {
    Bind,
    Configure,
    Accept,
    ReadRequest,
    WriteResponse,
    Shutdown,
    Protocol,
}

impl FixtureStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Bind => "bind",
            Self::Configure => "configure",
            Self::Accept => "accept",
            Self::ReadRequest => "read-request",
            Self::WriteResponse => "write-response",
            Self::Shutdown => "shutdown",
            Self::Protocol => "protocol",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FixtureFailure {
    Io(io::ErrorKind),
    Timeout,
    RequestTooLarge,
    ResponseTooLarge,
    TruncatedRequest,
    InvalidContentLength,
    BodyLengthMismatch,
    DuplicateOperation,
    UnexpectedConnection,
    Poisoned,
    InvalidWatchdog,
}

/// A redacted fixture failure. It deliberately records only the protocol
/// stage and a closed failure kind, never request or response bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FixtureError {
    stage: FixtureStage,
    failure: FixtureFailure,
}

impl FixtureError {
    const fn new(stage: FixtureStage, failure: FixtureFailure) -> Self {
        Self { stage, failure }
    }

    pub(super) const fn stage(&self) -> &'static str {
        self.stage.as_str()
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Responses fixture failed during {}: {:?}",
            self.stage.as_str(),
            self.failure
        )
    }
}

impl Error for FixtureError {}

/// Test-thread-owned loopback fixture. No background accept thread exists, so
/// dropping it before a client connects cannot block. Every I/O operation uses
/// the same explicit watchdog and bounded byte counts.
pub(super) struct ResponsesFixture {
    listener: mio::net::TcpListener,
    readiness: Mutex<ListenerReadiness>,
    address: SocketAddr,
    watchdog: Duration,
}

struct ListenerReadiness {
    poll: Poll,
    events: Events,
}

impl ResponsesFixture {
    pub(super) fn start() -> Result<Self, String> {
        Self::with_watchdog(super::NETWORK_TEST_TIMEOUT)
    }

    pub(super) fn with_watchdog(watchdog: Duration) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
            FixtureError::new(FixtureStage::Bind, FixtureFailure::Io(error.kind())).to_string()
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind())).to_string()
        })?;
        let address = listener.local_addr().map_err(|error| {
            FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind())).to_string()
        })?;
        let mut listener = mio::net::TcpListener::from_std(listener);
        let poll = Poll::new().map_err(|error| {
            FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind())).to_string()
        })?;
        poll.registry()
            .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)
            .map_err(|error| {
                FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind()))
                    .to_string()
            })?;
        Ok(Self {
            listener,
            readiness: Mutex::new(ListenerReadiness {
                poll,
                events: Events::with_capacity(4),
            }),
            address,
            watchdog,
        })
    }

    pub(super) const fn address(&self) -> SocketAddr {
        self.address
    }

    pub(super) fn endpoint(&self) -> Result<super::ResponsesEndpoint, String> {
        super::ResponsesEndpoint::for_test(format!("http://{}/v1/responses", self.address))
            .map_err(|_| "test endpoint did not resolve".to_string())
    }

    pub(super) fn accept_request(&self) -> Result<ResponsesExchange, FixtureError> {
        self.accept_request_within(self.watchdog)
    }

    pub(super) fn accept_request_within(
        &self,
        watchdog: Duration,
    ) -> Result<ResponsesExchange, FixtureError> {
        self.accept_connection_within(watchdog)?.capture_request()
    }

    pub(super) fn accept_connection(&self) -> Result<AcceptedConnection, FixtureError> {
        self.accept_connection_within(self.watchdog)
    }

    fn accept_connection_within(
        &self,
        watchdog: Duration,
    ) -> Result<AcceptedConnection, FixtureError> {
        let deadline = deadline_after(watchdog)?;
        let (stream, _) = loop {
            match self.listener.accept() {
                Ok((stream, address)) => {
                    let stream: TcpStream = stream.into();
                    stream.set_nonblocking(false).map_err(|error| {
                        FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind()))
                    })?;
                    break (stream, address);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(FixtureError::new(
                            FixtureStage::Accept,
                            FixtureFailure::Timeout,
                        ));
                    }
                    let mut readiness = self.readiness.lock().map_err(|_| {
                        FixtureError::new(FixtureStage::Accept, FixtureFailure::Poisoned)
                    })?;
                    let ListenerReadiness { poll, events } = &mut *readiness;
                    events.clear();
                    match poll.poll(events, Some(remaining)) {
                        Ok(()) if events.is_empty() => {
                            return Err(FixtureError::new(
                                FixtureStage::Accept,
                                FixtureFailure::Timeout,
                            ));
                        }
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            return Err(FixtureError::new(
                                FixtureStage::Accept,
                                FixtureFailure::Io(error.kind()),
                            ));
                        }
                    }
                }
                Err(error) => {
                    return Err(FixtureError::new(
                        FixtureStage::Accept,
                        FixtureFailure::Io(error.kind()),
                    ));
                }
            }
        };

        configure_stream(&stream, deadline)?;
        Ok(AcceptedConnection {
            stream: Some(stream),
            deadline,
        })
    }

    /// Checks for an already-dispatched extra physical request. Callers must
    /// first establish that the owner has quiesced; this is not a substitute
    /// for a positive completion milestone.
    pub(super) fn assert_no_pending_request(&self) -> Result<(), FixtureError> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                let stream: TcpStream = stream.into();
                let _ignored = stream.shutdown(Shutdown::Both);
                Err(FixtureError::new(
                    FixtureStage::Protocol,
                    FixtureFailure::UnexpectedConnection,
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(FixtureError::new(
                FixtureStage::Accept,
                FixtureFailure::Io(error.kind()),
            )),
        }
    }
}

pub(super) struct AcceptedConnection {
    stream: Option<TcpStream>,
    deadline: Instant,
}

impl AcceptedConnection {
    pub(super) fn capture_request(mut self) -> Result<ResponsesExchange, FixtureError> {
        let mut stream = self.stream.take().ok_or_else(|| {
            FixtureError::new(FixtureStage::Protocol, FixtureFailure::DuplicateOperation)
        })?;
        let request = CapturedRequest {
            bytes: read_request(&mut stream, self.deadline)?,
        };
        Ok(ResponsesExchange {
            stream: Some(stream),
            request,
            response_state: ResponseState::Awaiting,
            deadline: self.deadline,
        })
    }
}

impl Drop for AcceptedConnection {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ignored = stream.shutdown(Shutdown::Both);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseState {
    Awaiting,
    HeadersSent { content_length: usize },
    Closed,
}

pub(super) struct ResponsesExchange {
    stream: Option<TcpStream>,
    request: CapturedRequest,
    response_state: ResponseState,
    deadline: Instant,
}

impl ResponsesExchange {
    pub(super) const fn request(&self) -> &CapturedRequest {
        &self.request
    }

    pub(super) fn respond_raw(&mut self, response: &[u8]) -> Result<(), FixtureError> {
        if self.response_state != ResponseState::Awaiting {
            return Err(FixtureError::new(
                FixtureStage::Protocol,
                FixtureFailure::DuplicateOperation,
            ));
        }
        if response.len() > RESPONSE_BYTE_LIMIT {
            return Err(FixtureError::new(
                FixtureStage::WriteResponse,
                FixtureFailure::ResponseTooLarge,
            ));
        }
        self.write_all(response)?;
        self.shutdown(Shutdown::Write)?;
        self.response_state = ResponseState::Closed;
        Ok(())
    }

    pub(super) fn respond(
        &mut self,
        status: &str,
        headers: &[&str],
        body: &[u8],
    ) -> Result<(), FixtureError> {
        self.send_headers(status, headers, body.len())?;
        self.send_body(body)
    }

    pub(super) fn send_headers(
        &mut self,
        status: &str,
        headers: &[&str],
        content_length: usize,
    ) -> Result<(), FixtureError> {
        if self.response_state != ResponseState::Awaiting {
            return Err(FixtureError::new(
                FixtureStage::Protocol,
                FixtureFailure::DuplicateOperation,
            ));
        }
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\nConnection: close\r\n"
        )
        .into_bytes();
        for header in headers {
            response.extend_from_slice(header.as_bytes());
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(b"\r\n");
        if response.len() > RESPONSE_BYTE_LIMIT {
            return Err(FixtureError::new(
                FixtureStage::WriteResponse,
                FixtureFailure::ResponseTooLarge,
            ));
        }
        self.write_all(&response)?;
        self.response_state = ResponseState::HeadersSent { content_length };
        Ok(())
    }

    pub(super) fn send_body(&mut self, body: &[u8]) -> Result<(), FixtureError> {
        let ResponseState::HeadersSent { content_length } = self.response_state else {
            return Err(FixtureError::new(
                FixtureStage::Protocol,
                FixtureFailure::DuplicateOperation,
            ));
        };
        if body.len() != content_length {
            return Err(FixtureError::new(
                FixtureStage::Protocol,
                FixtureFailure::BodyLengthMismatch,
            ));
        }
        if body.len() > RESPONSE_BYTE_LIMIT {
            return Err(FixtureError::new(
                FixtureStage::WriteResponse,
                FixtureFailure::ResponseTooLarge,
            ));
        }
        self.write_all(body)?;
        self.shutdown(Shutdown::Write)?;
        self.response_state = ResponseState::Closed;
        Ok(())
    }

    pub(super) fn close_without_response(mut self) -> Result<(), FixtureError> {
        self.shutdown(Shutdown::Both)?;
        self.response_state = ResponseState::Closed;
        Ok(())
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), FixtureError> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            FixtureError::new(FixtureStage::Protocol, FixtureFailure::DuplicateOperation)
        })?;
        while !bytes.is_empty() {
            // A partial write or EINTR must consume the same absolute
            // watchdog; `Write::write_all` would reuse one syscall timeout.
            configure_stream(stream, self.deadline)?;
            match stream.write(bytes) {
                Ok(0) => {
                    return Err(FixtureError::new(
                        FixtureStage::WriteResponse,
                        FixtureFailure::Io(io::ErrorKind::WriteZero),
                    ));
                }
                Ok(written) => bytes = &bytes[written..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    let failure = if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) {
                        FixtureFailure::Timeout
                    } else {
                        FixtureFailure::Io(error.kind())
                    };
                    return Err(FixtureError::new(FixtureStage::WriteResponse, failure));
                }
            }
        }
        Ok(())
    }

    fn shutdown(&mut self, direction: Shutdown) -> Result<(), FixtureError> {
        let Some(stream) = self.stream.as_ref() else {
            return Ok(());
        };
        match stream.shutdown(direction) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotConnected => Ok(()),
            Err(error) => Err(FixtureError::new(
                FixtureStage::Shutdown,
                FixtureFailure::Io(error.kind()),
            )),
        }
    }
}

pub(super) struct CapturedRequest {
    bytes: Vec<u8>,
}

impl CapturedRequest {
    pub(super) fn to_vec(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

impl fmt::Debug for CapturedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedRequest")
            .field("byte_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ResponsesExchange {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ignored = stream.shutdown(Shutdown::Both);
        }
        self.response_state = ResponseState::Closed;
    }
}

fn deadline_after(watchdog: Duration) -> Result<Instant, FixtureError> {
    Instant::now()
        .checked_add(watchdog)
        .ok_or_else(|| FixtureError::new(FixtureStage::Configure, FixtureFailure::InvalidWatchdog))
}

fn configure_stream(stream: &TcpStream, deadline: Instant) -> Result<(), FixtureError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(FixtureError::new(
            FixtureStage::Configure,
            FixtureFailure::Timeout,
        ));
    }
    stream.set_read_timeout(Some(remaining)).map_err(|error| {
        FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind()))
    })?;
    stream.set_write_timeout(Some(remaining)).map_err(|error| {
        FixtureError::new(FixtureStage::Configure, FixtureFailure::Io(error.kind()))
    })?;
    Ok(())
}

fn read_request(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, FixtureError> {
    let mut request = Vec::with_capacity(8 * 1024);
    let mut expected_bytes = None;
    let mut chunk = [0_u8; 4 * 1024];
    loop {
        if expected_bytes.is_some_and(|expected| request.len() >= expected) {
            return Ok(request);
        }
        if request.len() >= REQUEST_BYTE_LIMIT {
            return Err(FixtureError::new(
                FixtureStage::ReadRequest,
                FixtureFailure::RequestTooLarge,
            ));
        }
        configure_stream(stream, deadline)?;
        let count = stream.read(&mut chunk).map_err(|error| {
            let failure = if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) {
                FixtureFailure::Timeout
            } else {
                FixtureFailure::Io(error.kind())
            };
            FixtureError::new(FixtureStage::ReadRequest, failure)
        })?;
        if count == 0 {
            return Err(FixtureError::new(
                FixtureStage::ReadRequest,
                FixtureFailure::TruncatedRequest,
            ));
        }
        let next_len = request.len().checked_add(count).ok_or_else(|| {
            FixtureError::new(FixtureStage::ReadRequest, FixtureFailure::RequestTooLarge)
        })?;
        if next_len > REQUEST_BYTE_LIMIT {
            return Err(FixtureError::new(
                FixtureStage::ReadRequest,
                FixtureFailure::RequestTooLarge,
            ));
        }
        request.extend_from_slice(&chunk[..count]);
        if expected_bytes.is_none()
            && let Some(headers_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4)
        {
            let content_length = request_body_length(&request[..headers_end])?;
            let total = headers_end.checked_add(content_length).ok_or_else(|| {
                FixtureError::new(FixtureStage::ReadRequest, FixtureFailure::RequestTooLarge)
            })?;
            if total > REQUEST_BYTE_LIMIT {
                return Err(FixtureError::new(
                    FixtureStage::ReadRequest,
                    FixtureFailure::RequestTooLarge,
                ));
            }
            expected_bytes = Some(total);
        }
    }
}

fn content_length(headers: &[u8]) -> Result<Option<usize>, FixtureError> {
    let headers = std::str::from_utf8(headers).map_err(|_| {
        FixtureError::new(
            FixtureStage::ReadRequest,
            FixtureFailure::InvalidContentLength,
        )
    })?;
    let mut lengths = headers.lines().filter_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then_some(value.trim())
    });
    let Some(value) = lengths.next() else {
        return Ok(None);
    };
    if lengths.next().is_some() {
        return Err(FixtureError::new(
            FixtureStage::ReadRequest,
            FixtureFailure::InvalidContentLength,
        ));
    }
    value.parse::<usize>().map(Some).map_err(|_| {
        FixtureError::new(
            FixtureStage::ReadRequest,
            FixtureFailure::InvalidContentLength,
        )
    })
}

fn request_body_length(headers: &[u8]) -> Result<usize, FixtureError> {
    if let Some(length) = content_length(headers)? {
        return Ok(length);
    }
    if headers.starts_with(b"CONNECT ") {
        return Ok(0);
    }
    Err(FixtureError::new(
        FixtureStage::ReadRequest,
        FixtureFailure::InvalidContentLength,
    ))
}
