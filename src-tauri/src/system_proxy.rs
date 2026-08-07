//! System-proxy routing for the OpenAI Realtime TCP tunnel.
//!
//! OpenAI authentication belongs to the later TLS/WebSocket handshake. The
//! plaintext HTTP proxy sees only a CONNECT request and, when configured, its
//! own `Proxy-Authorization` header.

use crate::error::{AppError, AppResult};
use crate::host_resolver::{HostResolutionError, HostResolver};
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use std::env::VarError;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::handshake::client::Request;
use tungstenite::http::{HeaderValue, Uri};

const PROXY_CONNECT_BUDGET: Duration = Duration::from_secs(10);
const MAX_CONNECT_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
const MAX_CONNECT_RESPONSE_HEADERS: usize = 128;

pub(super) fn connect_with_system_proxy(
    request: &Request,
    resolver: &HostResolver,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<TcpStream> {
    let deadline = Instant::now() + PROXY_CONNECT_BUDGET;
    ensure_connection_not_cancelled(is_cancelled)?;
    // Read the environment and OS settings for every session so changing the
    // system proxy does not require restarting the application.
    let matcher = system_proxy_matcher()?;
    connect_with_matcher_until(request, &matcher, resolver, deadline, is_cancelled)
}

fn system_proxy_matcher() -> AppResult<Matcher> {
    let https_proxy = first_environment_value(&["HTTPS_PROXY", "https_proxy"])?;
    let all_proxy = first_environment_value(&["ALL_PROXY", "all_proxy"])?;
    let no_proxy = first_environment_value(&["NO_PROXY", "no_proxy"])?;
    if let Some(proxy) = https_proxy
        .filter(|value| !value.trim().is_empty())
        .or_else(|| all_proxy.filter(|value| !value.trim().is_empty()))
    {
        return matcher_for_configured_https_proxy(&proxy, no_proxy.as_deref());
    }

    system_proxy_matcher_without_explicit_https(no_proxy)
}

fn first_environment_value(names: &[&str]) -> AppResult<Option<String>> {
    for name in names {
        match std::env::var(name) {
            Ok(value) => return Ok(Some(value)),
            Err(VarError::NotPresent) => {}
            Err(VarError::NotUnicode(_)) => {
                return Err(AppError::stt_network(format!(
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
        return Err(AppError::stt_network(
            "A proxy was selected for OpenAI Realtime but its address is empty.",
        ));
    }
    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let uri = candidate.parse::<Uri>().map_err(|error| {
        AppError::stt_network(format!(
            "The configured OpenAI Realtime proxy address is invalid: {error}"
        ))
    })?;
    let scheme = uri.scheme_str().ok_or_else(|| {
        AppError::stt_network("The configured OpenAI Realtime proxy has no URI scheme.")
    })?;
    if scheme != "http" {
        return Err(AppError::stt_network(format!(
            "The selected system proxy scheme '{scheme}' is not supported for OpenAI Realtime; use an HTTP CONNECT proxy."
        )));
    }
    if uri.host().is_none() {
        return Err(AppError::stt_network(
            "The configured OpenAI Realtime proxy has no host name.",
        ));
    }
    Ok(candidate)
}

fn normalized_no_proxy(value: &str) -> String {
    value
        .split([';', ','])
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case("<local>"))
        .map(|entry| entry.strip_prefix("*.").unwrap_or(entry))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(any(target_os = "macos", test))]
#[path = "system_proxy/macos.rs"]
mod macos;

#[cfg(any(target_os = "windows", test))]
#[path = "system_proxy/windows.rs"]
mod windows;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_proxy_matcher_without_explicit_https(no_proxy: Option<String>) -> AppResult<Matcher> {
    // Linux has no additional system source, so reaching this branch means
    // HTTPS_PROXY/ALL_PROXY explicitly selected no proxy.
    Ok(direct_matcher(no_proxy.as_deref()))
}

#[cfg(target_os = "macos")]
fn system_proxy_matcher_without_explicit_https(no_proxy: Option<String>) -> AppResult<Matcher> {
    macos::system_proxy_matcher(no_proxy)
}

#[cfg(target_os = "windows")]
fn system_proxy_matcher_without_explicit_https(no_proxy: Option<String>) -> AppResult<Matcher> {
    windows::system_proxy_matcher(no_proxy)
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
            return Err(AppError::stt(
                "OpenAI Realtime system-proxy routing requires a wss URI.",
            ));
        }
        let host = request.uri().host().ok_or_else(|| {
            AppError::stt("OpenAI Realtime WebSocket URI did not include a host name.")
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
        AppError::stt("OpenAI Realtime WebSocket URI did not include an authority.")
    })?;
    Uri::builder()
        .scheme("https")
        .authority(authority)
        .path_and_query("/")
        .build()
        .map_err(|error| {
            AppError::stt(format!(
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
        return Err(AppError::stt_network(format!(
            "The selected system proxy scheme '{scheme}' is not supported for OpenAI Realtime; use an HTTP CONNECT proxy."
        )));
    }
    let proxy_host = proxy.uri().host().ok_or_else(|| {
        AppError::stt_network("The selected system proxy did not include a host name.")
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

    write_connect_request(&mut stream, &target.authority, proxy.basic_auth(), deadline)?;
    let status = read_connect_response(&mut stream, deadline)?;
    match status {
        200..=299 => Ok(stream),
        407 => Err(AppError::stt_network(
            "The system proxy rejected OpenAI Realtime with HTTP 407; check the proxy authentication settings.",
        )),
        other => Err(AppError::stt_network(format!(
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
        let remaining = remaining_budget(deadline, destination)?;
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                stream.set_nodelay(true).map_err(|error| {
                    AppError::stt_network(format!(
                        "Failed to configure the connection to {destination}: {error}"
                    ))
                })?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }

    let detail = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "DNS returned no addresses".to_string());
    Err(AppError::stt_network(format!(
        "Could not connect to {destination} within {} seconds: {detail}",
        PROXY_CONNECT_BUDGET.as_secs()
    )))
}

fn ensure_connection_not_cancelled(is_cancelled: &dyn Fn() -> bool) -> AppResult<()> {
    if is_cancelled() {
        Err(AppError::stt_network(
            "OpenAI Realtime connection was cancelled during startup.",
        ))
    } else {
        Ok(())
    }
}

fn map_resolution_error(destination: &str, error: HostResolutionError) -> AppError {
    let detail = match error {
        HostResolutionError::Cancelled => {
            format!("Hostname resolution for {destination} was cancelled.")
        }
        HostResolutionError::DeadlineExceeded => {
            format!(
                "Hostname resolution for {destination} timed out before the connection deadline."
            )
        }
        HostResolutionError::LookupFailed(error) => {
            format!("Failed to resolve {destination}: {error}")
        }
        HostResolutionError::WorkerUnavailable(error) => {
            format!("The hostname resolver was unavailable for {destination}: {error}")
        }
        HostResolutionError::QueueFull => {
            format!("The hostname resolver queue was full while resolving {destination}.")
        }
    };
    AppError::stt_network(detail)
}

fn write_connect_request(
    stream: &mut TcpStream,
    target_authority: &str,
    proxy_authorization: Option<&HeaderValue>,
    deadline: Instant,
) -> AppResult<()> {
    let mut request = format!(
        "CONNECT {target_authority} HTTP/1.1\r\nHost: {target_authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if let Some(value) = proxy_authorization {
        let value = value.to_str().map_err(|error| {
            AppError::stt_network(format!(
                "The system proxy authorization value is invalid: {error}"
            ))
        })?;
        request.push_str("Proxy-Authorization: ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    set_write_budget(stream, deadline, "send the system proxy CONNECT request")?;
    stream.write_all(request.as_bytes()).map_err(|error| {
        AppError::stt_network(format!(
            "Failed to send the system proxy CONNECT request: {error}"
        ))
    })
}

fn read_connect_response(stream: &mut TcpStream, deadline: Instant) -> AppResult<u16> {
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
            return Err(AppError::stt_network(
                "The system proxy CONNECT response header exceeded 16 KiB.",
            ));
        }

        set_read_budget(stream, deadline, "read the system proxy CONNECT response")?;
        let remaining_capacity = MAX_CONNECT_RESPONSE_HEADER_BYTES - response.len();
        let read_length = remaining_capacity.min(chunk.len());
        let count = stream.read(&mut chunk[..read_length]).map_err(|error| {
            AppError::stt_network(format!(
                "Failed to read the system proxy CONNECT response: {error}"
            ))
        })?;
        if count == 0 {
            return Err(AppError::stt_network(
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
            AppError::stt_network("The system proxy CONNECT response omitted its HTTP status.")
        }),
        Ok(httparse::Status::Partial) => Err(AppError::stt_network(
            "The system proxy CONNECT response header was incomplete.",
        )),
        Err(error) => Err(AppError::stt_network(format!(
            "The system proxy returned an invalid CONNECT response: {error}"
        ))),
    }
}

fn set_write_budget(stream: &TcpStream, deadline: Instant, operation: &str) -> AppResult<()> {
    let remaining = remaining_budget(deadline, operation)?;
    stream.set_write_timeout(Some(remaining)).map_err(|error| {
        AppError::stt_network(format!(
            "Failed to configure the system proxy write timeout: {error}"
        ))
    })
}

fn set_read_budget(stream: &TcpStream, deadline: Instant, operation: &str) -> AppResult<()> {
    let remaining = remaining_budget(deadline, operation)?;
    stream.set_read_timeout(Some(remaining)).map_err(|error| {
        AppError::stt_network(format!(
            "Failed to configure the system proxy read timeout: {error}"
        ))
    })
}

fn remaining_budget(deadline: Instant, operation: &str) -> AppResult<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(AppError::stt_network(format!(
            "Timed out while trying to {operation}."
        )))
    } else {
        Ok(remaining)
    }
}

#[cfg(test)]
#[path = "system_proxy_tests.rs"]
mod tests;
