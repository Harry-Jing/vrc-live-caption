//! System-proxy routing for the OpenAI Realtime TCP tunnel.
//!
//! OpenAI authentication belongs to the later TLS/WebSocket handshake. The
//! plaintext HTTP proxy sees only a CONNECT request and, when configured, its
//! own `Proxy-Authorization` header.

use crate::error::{AppError, AppResult};
use crate::host_resolver::{HostResolutionError, HostResolver};
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use mio::net::TcpStream as MioTcpStream;
use mio::{Events, Interest, Poll, Token};
use std::env::VarError;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::handshake::client::Request;
use tungstenite::http::{HeaderValue, Uri};

const PROXY_CONNECT_BUDGET: Duration = Duration::from_secs(10);
const CONNECTION_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TCP_CONNECT_TOKEN: Token = Token(0);
const MAX_CONNECT_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
const MAX_CONNECT_RESPONSE_HEADERS: usize = 128;

pub(super) fn connect_with_system_proxy(
    request: &Request,
    resolver: &HostResolver,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<TcpStream> {
    let deadline = Instant::now() + PROXY_CONNECT_BUDGET;
    ensure_connection_not_cancelled(is_cancelled)?;
    // Read the environment and OS settings for every connection attempt so
    // changing the system proxy does not require restarting the application.
    let match_uri = https_proxy_match_uri(request.uri())?;
    let matcher = system_proxy_matcher(&match_uri)?;
    connect_with_matcher_until(request, &matcher, resolver, deadline, is_cancelled)
}

fn system_proxy_matcher(target: &Uri) -> AppResult<Matcher> {
    let https_proxy = first_environment_value(&["HTTPS_PROXY", "https_proxy"])?;
    let all_proxy = first_environment_value(&["ALL_PROXY", "all_proxy"])?;
    let explicit_proxy = https_proxy
        .filter(|value| !value.trim().is_empty())
        .or_else(|| all_proxy.filter(|value| !value.trim().is_empty()));

    matcher_for_proxy_sources(
        explicit_proxy,
        || first_environment_value(&["NO_PROXY", "no_proxy"]),
        || system_proxy_matcher_without_explicit_https(target),
    )
}

fn matcher_for_proxy_sources(
    explicit_proxy: Option<String>,
    no_proxy: impl FnOnce() -> AppResult<Option<String>>,
    system_matcher: impl FnOnce() -> AppResult<Matcher>,
) -> AppResult<Matcher> {
    if let Some(proxy) = explicit_proxy {
        let no_proxy = no_proxy()?;
        matcher_for_configured_https_proxy(&proxy, no_proxy.as_deref())
    } else {
        system_matcher()
    }
}

fn first_environment_value(names: &[&str]) -> AppResult<Option<String>> {
    for name in names {
        match std::env::var(name) {
            Ok(value) => return Ok(Some(value)),
            Err(VarError::NotPresent) => {}
            Err(VarError::NotUnicode(_)) => {
                return Err(AppError::recognition_network_terminal(format!(
                    "The {name} proxy setting is not valid Unicode; refusing a direct OpenAI connection."
                )));
            }
        }
    }
    Ok(None)
}

fn matcher_for_configured_https_proxy(proxy: &str, no_proxy: Option<&str>) -> AppResult<Matcher> {
    let proxy = normalized_http_proxy_uri(proxy)?;
    let mut builder = Matcher::builder().https(proxy);
    if let Some(no_proxy) = no_proxy {
        builder = builder.no(normalized_no_proxy(no_proxy));
    }
    Ok(builder.build())
}

fn normalized_http_proxy_uri(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::recognition_network_terminal(
            "A proxy was selected for OpenAI Realtime but its address is empty.",
        ));
    }
    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let uri = candidate.parse::<Uri>().map_err(|error| {
        AppError::recognition_network_terminal(format!(
            "The configured OpenAI Realtime proxy address is invalid: {error}"
        ))
    })?;
    let scheme = uri.scheme_str().ok_or_else(|| {
        AppError::recognition_network_terminal(
            "The configured OpenAI Realtime proxy has no URI scheme.",
        )
    })?;
    if scheme != "http" {
        return Err(AppError::recognition_network_terminal(format!(
            "The selected system proxy scheme '{scheme}' is not supported for OpenAI Realtime; use an HTTP CONNECT proxy."
        )));
    }
    if uri.host().is_none() {
        return Err(AppError::recognition_network_terminal(
            "The configured OpenAI Realtime proxy has no host name.",
        ));
    }
    Ok(candidate)
}

fn normalized_no_proxy(value: &str) -> String {
    value
        .split(|character: char| matches!(character, ';' | ',') || character.is_ascii_whitespace())
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case("<local>"))
        .map(|entry| entry.strip_prefix("*.").unwrap_or(entry))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(any(target_os = "macos", test))]
mod macos;

#[cfg(any(target_os = "windows", test))]
mod windows;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_proxy_matcher_without_explicit_https(_target: &Uri) -> AppResult<Matcher> {
    // Linux has no additional supported system-proxy source. Reaching this branch
    // means no non-empty environment proxy was selected, so connect directly;
    // NO_PROXY alone does not select an environment route.
    Ok(direct_matcher(None))
}

#[cfg(target_os = "macos")]
fn system_proxy_matcher_without_explicit_https(target: &Uri) -> AppResult<Matcher> {
    macos::system_proxy_matcher(&target.to_string())
}

#[cfg(target_os = "windows")]
fn system_proxy_matcher_without_explicit_https(_target: &Uri) -> AppResult<Matcher> {
    windows::system_proxy_matcher(None)
}

fn direct_matcher(no_proxy: Option<&str>) -> Matcher {
    let mut builder = Matcher::builder();
    if let Some(no_proxy) = no_proxy {
        builder = builder.no(normalized_no_proxy(no_proxy));
    }
    builder.build()
}

#[cfg(test)]
fn connect_with_matcher(request: &Request, matcher: &Matcher) -> AppResult<TcpStream> {
    connect_with_matcher_until(
        request,
        matcher,
        &HostResolver::default(),
        Instant::now() + PROXY_CONNECT_BUDGET,
        &|| false,
    )
}

fn connect_with_matcher_until(
    request: &Request,
    matcher: &Matcher,
    resolver: &HostResolver,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<TcpStream> {
    let target = Target::from_request(request)?;
    let match_uri = https_proxy_match_uri(request.uri())?;

    let Some(proxy) = matcher.intercept(&match_uri) else {
        return connect_tcp(
            resolver,
            &target.host,
            target.port,
            deadline,
            "OpenAI Realtime",
            is_cancelled,
        );
    };

    connect_http_proxy(&target, &proxy, resolver, deadline, is_cancelled)
}

struct Target {
    host: String,
    port: u16,
    authority: String,
}

impl Target {
    fn from_request(request: &Request) -> AppResult<Self> {
        if request.uri().scheme_str() != Some("wss") {
            return Err(AppError::recognition(
                "OpenAI Realtime system-proxy routing requires a wss URI.",
            ));
        }
        let host = request.uri().host().ok_or_else(|| {
            AppError::recognition("OpenAI Realtime WebSocket URI did not include a host name.")
        })?;
        let port = request.uri().port_u16().unwrap_or(443);
        Ok(Self {
            host: host.trim_matches(['[', ']']).to_string(),
            port,
            authority: authority_with_port(host, port),
        })
    }
}

fn https_proxy_match_uri(websocket_uri: &Uri) -> AppResult<Uri> {
    let authority = websocket_uri.authority().cloned().ok_or_else(|| {
        AppError::recognition("OpenAI Realtime WebSocket URI did not include an authority.")
    })?;
    Uri::builder()
        .scheme("https")
        .authority(authority)
        .path_and_query("/")
        .build()
        .map_err(|error| {
            AppError::recognition(format!(
                "Failed to map the OpenAI Realtime URI for system-proxy matching: {error}"
            ))
        })
}

fn authority_with_port(host: &str, port: u16) -> String {
    if host.starts_with('[') || !host.contains(':') {
        format!("{host}:{port}")
    } else {
        format!("[{host}]:{port}")
    }
}

fn connect_http_proxy(
    target: &Target,
    proxy: &Intercept,
    resolver: &HostResolver,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<TcpStream> {
    let scheme = proxy.uri().scheme_str().unwrap_or("http");
    if scheme != "http" {
        return Err(AppError::recognition_network_terminal(format!(
            "The selected system proxy scheme '{scheme}' is not supported for OpenAI Realtime; use an HTTP CONNECT proxy."
        )));
    }
    let proxy_host = proxy.uri().host().ok_or_else(|| {
        AppError::recognition_network_terminal(
            "The selected system proxy did not include a host name.",
        )
    })?;
    let proxy_host = proxy_host.trim_matches(['[', ']']);
    let proxy_port = proxy.uri().port_u16().unwrap_or(80);
    let mut stream = connect_tcp(
        resolver,
        proxy_host,
        proxy_port,
        deadline,
        "the selected system proxy",
        is_cancelled,
    )?;

    write_connect_request(
        &mut stream,
        &target.authority,
        proxy.basic_auth(),
        deadline,
        is_cancelled,
    )?;
    let status = read_connect_response(&mut stream, deadline, is_cancelled)?;
    validate_proxy_connect_status(status)?;
    Ok(stream)
}

fn validate_proxy_connect_status(status: u16) -> AppResult<()> {
    match status {
        200..=299 => Ok(()),
        407 => Err(AppError::recognition_network_terminal(
            "The system proxy rejected OpenAI Realtime with HTTP 407; check the proxy authentication settings.",
        )),
        408 | 429 | 502..=504 => Err(AppError::recognition_network_retryable(format!(
            "The system proxy temporarily could not open the OpenAI Realtime tunnel: HTTP {status}."
        ))),
        other => Err(AppError::recognition_network_terminal(format!(
            "The system proxy could not open the OpenAI Realtime tunnel: HTTP {other}."
        ))),
    }
}

fn connect_tcp(
    resolver: &HostResolver,
    host: &str,
    port: u16,
    deadline: Instant,
    destination: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<TcpStream> {
    let addresses = resolver
        .resolve_until(host, port, deadline, is_cancelled)
        .map_err(|error| map_resolution_error(destination, error))?;
    let mut last_error = None;
    for address in addresses {
        ensure_connection_not_cancelled(is_cancelled)?;
        let mut stream = match MioTcpStream::connect(address) {
            Ok(stream) => stream,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let mut poll = Poll::new().map_err(|error| {
            AppError::recognition_network_terminal(format!(
                "Failed to create a connection poller for {destination}: {error}"
            ))
        })?;
        poll.registry()
            .register(&mut stream, TCP_CONNECT_TOKEN, Interest::WRITABLE)
            .map_err(|error| {
                AppError::recognition_network_terminal(format!(
                    "Failed to monitor the connection to {destination}: {error}"
                ))
            })?;
        let mut events = Events::with_capacity(4);

        loop {
            ensure_connection_not_cancelled(is_cancelled)?;
            let remaining = remaining_budget(deadline, destination)?;
            let wait = remaining.min(CONNECTION_CANCEL_POLL_INTERVAL);
            match poll.poll(&mut events, Some(wait)) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(AppError::recognition_network_retryable(format!(
                        "Failed while waiting to connect to {destination}: {error}"
                    )));
                }
            }
            if !events
                .iter()
                .any(|event| event.token() == TCP_CONNECT_TOKEN)
            {
                continue;
            }
            match stream.take_error() {
                Ok(Some(error)) => {
                    last_error = Some(error);
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    last_error = Some(error);
                    break;
                }
            }
            match stream.peer_addr() {
                Ok(_) => {
                    let stream = TcpStream::from(stream);
                    stream.set_nonblocking(false).map_err(|error| {
                        AppError::recognition_network_terminal(format!(
                            "Failed to restore blocking I/O for {destination}: {error}"
                        ))
                    })?;
                    stream.set_nodelay(true).map_err(|error| {
                        AppError::recognition_network_terminal(format!(
                            "Failed to configure the connection to {destination}: {error}"
                        ))
                    })?;
                    return Ok(stream);
                }
                // Mio documents that an in-progress connection may surface
                // here either as a portable not-connected kind or as a raw
                // platform EINPROGRESS value. `take_error` above is the
                // authoritative failure check, so any remaining peer lookup
                // error means this socket is not ready yet.
                Err(_) => continue,
            }
        }
    }

    let detail = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "DNS returned no addresses".to_string());
    Err(AppError::recognition_network_retryable(format!(
        "Could not connect to {destination} within {} seconds: {detail}",
        PROXY_CONNECT_BUDGET.as_secs()
    )))
}

fn ensure_connection_not_cancelled(is_cancelled: &dyn Fn() -> bool) -> AppResult<()> {
    if is_cancelled() {
        Err(AppError::recognition_network_terminal(
            "OpenAI Realtime connection was cancelled during startup.",
        ))
    } else {
        Ok(())
    }
}

fn map_resolution_error(destination: &str, error: HostResolutionError) -> AppError {
    let (detail, retryable) = match error {
        HostResolutionError::Cancelled => (
            format!("Hostname resolution for {destination} was cancelled."),
            false,
        ),
        HostResolutionError::DeadlineExceeded => (
            format!(
                "Hostname resolution for {destination} timed out before the connection deadline."
            ),
            true,
        ),
        HostResolutionError::LookupFailed(error) => {
            (format!("Failed to resolve {destination}: {error}"), true)
        }
        HostResolutionError::WorkerUnavailable(error) => (
            format!("The hostname resolver was unavailable for {destination}: {error}"),
            false,
        ),
        HostResolutionError::QueueFull => (
            format!("The hostname resolver queue was full while resolving {destination}."),
            true,
        ),
    };
    if retryable {
        AppError::recognition_network_retryable(detail)
    } else {
        AppError::recognition_network_terminal(detail)
    }
}

fn write_connect_request(
    stream: &mut TcpStream,
    target_authority: &str,
    proxy_authorization: Option<&HeaderValue>,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<()> {
    let mut request = format!(
        "CONNECT {target_authority} HTTP/1.1\r\nHost: {target_authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if let Some(value) = proxy_authorization {
        let value = value.to_str().map_err(|error| {
            AppError::recognition_network_terminal(format!(
                "The system proxy authorization value is invalid: {error}"
            ))
        })?;
        request.push_str("Proxy-Authorization: ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    let request = request.as_bytes();
    let mut written = 0;
    while written < request.len() {
        ensure_connection_not_cancelled(is_cancelled)?;
        set_write_budget(stream, deadline, "send the system proxy CONNECT request")?;
        match stream.write(&request[written..]) {
            Ok(0) => {
                return Err(AppError::recognition_network_retryable(
                    "The system proxy closed the connection before receiving the complete CONNECT request.",
                ));
            }
            Ok(count) => written += count,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                return Err(AppError::recognition_network_retryable(format!(
                    "Failed to send the system proxy CONNECT request: {error}"
                )));
            }
        }
    }
    Ok(())
}

fn read_connect_response(
    stream: &mut TcpStream,
    deadline: Instant,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<u16> {
    let mut response = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(header_length) = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
        {
            return parse_connect_status(&response[..header_length]);
        }
        if response.len() == MAX_CONNECT_RESPONSE_HEADER_BYTES {
            return Err(AppError::recognition_network_terminal(
                "The system proxy CONNECT response header exceeded 16 KiB.",
            ));
        }

        ensure_connection_not_cancelled(is_cancelled)?;
        set_read_budget(stream, deadline, "read the system proxy CONNECT response")?;
        let remaining_capacity = MAX_CONNECT_RESPONSE_HEADER_BYTES - response.len();
        let read_length = remaining_capacity.min(chunk.len());
        let count = match stream.read(&mut chunk[..read_length]) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(AppError::recognition_network_retryable(format!(
                    "Failed to read the system proxy CONNECT response: {error}"
                )));
            }
        };
        if count == 0 {
            return Err(AppError::recognition_network_retryable(
                "The system proxy closed the connection before completing its CONNECT response.",
            ));
        }
        response.extend_from_slice(&chunk[..count]);
    }
}

fn parse_connect_status(header: &[u8]) -> AppResult<u16> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_CONNECT_RESPONSE_HEADERS];
    let mut response = httparse::Response::new(&mut headers);
    match response.parse(header) {
        Ok(httparse::Status::Complete(_)) => response.code.ok_or_else(|| {
            AppError::recognition_network_terminal(
                "The system proxy CONNECT response omitted its HTTP status.",
            )
        }),
        Ok(httparse::Status::Partial) => Err(AppError::recognition_network_terminal(
            "The system proxy CONNECT response header was incomplete.",
        )),
        Err(error) => Err(AppError::recognition_network_terminal(format!(
            "The system proxy returned an invalid CONNECT response: {error}"
        ))),
    }
}

fn set_write_budget(stream: &TcpStream, deadline: Instant, operation: &str) -> AppResult<()> {
    let wait = remaining_budget(deadline, operation)?.min(CONNECTION_CANCEL_POLL_INTERVAL);
    stream.set_write_timeout(Some(wait)).map_err(|error| {
        AppError::recognition_network_terminal(format!(
            "Failed to configure the system proxy write timeout: {error}"
        ))
    })
}

fn set_read_budget(stream: &TcpStream, deadline: Instant, operation: &str) -> AppResult<()> {
    let wait = remaining_budget(deadline, operation)?.min(CONNECTION_CANCEL_POLL_INTERVAL);
    stream.set_read_timeout(Some(wait)).map_err(|error| {
        AppError::recognition_network_terminal(format!(
            "Failed to configure the system proxy read timeout: {error}"
        ))
    })
}

fn remaining_budget(deadline: Instant, operation: &str) -> AppResult<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(AppError::recognition_network_retryable(format!(
            "Timed out while trying to {operation}."
        )))
    } else {
        Ok(remaining)
    }
}

#[cfg(test)]
#[path = "system_proxy_tests.rs"]
mod tests;
