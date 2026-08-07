//! System-proxy routing for the OpenAI Realtime TCP tunnel.
//!
//! OpenAI authentication belongs to the later TLS/WebSocket handshake. The
//! plaintext HTTP proxy sees only a CONNECT request and, when configured, its
//! own `Proxy-Authorization` header.

use crate::error::{AppError, AppResult};
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use std::env::VarError;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use tungstenite::handshake::client::Request;
use tungstenite::http::{HeaderValue, Uri};

const PROXY_CONNECT_BUDGET: Duration = Duration::from_secs(10);
const MAX_CONNECT_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
const MAX_CONNECT_RESPONSE_HEADERS: usize = 128;

pub(super) fn connect_with_system_proxy(request: &Request) -> AppResult<TcpStream> {
    // Read the environment and OS settings for every session so changing the
    // system proxy does not require restarting the application.
    let matcher = system_proxy_matcher()?;
    connect_with_matcher(request, &matcher)
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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_proxy_matcher_without_explicit_https(no_proxy: Option<String>) -> AppResult<Matcher> {
    // Linux has no additional system source, so reaching this branch means
    // HTTPS_PROXY/ALL_PROXY explicitly selected no proxy.
    Ok(direct_matcher(no_proxy.as_deref()))
}

#[cfg(target_os = "macos")]
fn system_proxy_matcher_without_explicit_https(no_proxy: Option<String>) -> AppResult<Matcher> {
    use system_configuration::core_foundation::base::CFType;
    use system_configuration::core_foundation::dictionary::CFDictionary;
    use system_configuration::core_foundation::number::CFNumber;
    use system_configuration::core_foundation::string::CFString;
    use system_configuration::dynamic_store::SCDynamicStoreBuilder;

    fn number(proxies: &CFDictionary<CFString, CFType>, key: &str) -> AppResult<Option<i32>> {
        let key_value = CFString::new(key);
        let Some(value) = proxies.find(&key_value) else {
            return Ok(None);
        };
        value
            .downcast::<CFNumber>()
            .and_then(|number| number.to_i32())
            .map(Some)
            .ok_or_else(|| {
                AppError::stt_network(format!(
                    "macOS system proxy field {key} has an invalid type or value; refusing a direct OpenAI connection."
                ))
            })
    }

    fn string(proxies: &CFDictionary<CFString, CFType>, key: &str) -> AppResult<Option<String>> {
        let key_value = CFString::new(key);
        let Some(value) = proxies.find(&key_value) else {
            return Ok(None);
        };
        value
            .downcast::<CFString>()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                AppError::stt_network(format!(
                    "macOS system proxy field {key} has an invalid type; refusing a direct OpenAI connection."
                ))
            })
    }

    let store = SCDynamicStoreBuilder::new("vrc-live-caption")
        .build()
        .ok_or_else(|| {
            AppError::stt_network(
                "macOS system proxy settings could not be opened; refusing a direct OpenAI connection.",
            )
        })?;
    let proxies = store.get_proxies().ok_or_else(|| {
        AppError::stt_network(
            "macOS system proxy settings could not be read; refusing a direct OpenAI connection.",
        )
    })?;
    let settings = MacProxySettings {
        https_enabled: number(&proxies, "HTTPSEnable")?.unwrap_or(0),
        https_host: string(&proxies, "HTTPSProxy")?,
        https_port: number(&proxies, "HTTPSPort")?,
        socks_enabled: number(&proxies, "SOCKSEnable")?.unwrap_or(0),
        pac_enabled: number(&proxies, "ProxyAutoConfigEnable")?.unwrap_or(0),
        auto_discovery_enabled: number(&proxies, "ProxyAutoDiscoveryEnable")?.unwrap_or(0),
    };
    matcher_for_macos_proxy_settings(&settings, no_proxy.as_deref())
}

#[cfg(any(target_os = "macos", test))]
struct MacProxySettings {
    https_enabled: i32,
    https_host: Option<String>,
    https_port: Option<i32>,
    socks_enabled: i32,
    pac_enabled: i32,
    auto_discovery_enabled: i32,
}

#[cfg(any(target_os = "macos", test))]
fn matcher_for_macos_proxy_settings(
    settings: &MacProxySettings,
    no_proxy: Option<&str>,
) -> AppResult<Matcher> {
    for (name, value) in [
        ("HTTPSEnable", settings.https_enabled),
        ("SOCKSEnable", settings.socks_enabled),
        ("ProxyAutoConfigEnable", settings.pac_enabled),
        ("ProxyAutoDiscoveryEnable", settings.auto_discovery_enabled),
    ] {
        if !matches!(value, 0 | 1) {
            return Err(AppError::stt_network(format!(
                "macOS system proxy field {name} has invalid value {value}; refusing a direct OpenAI connection."
            )));
        }
    }
    if settings.pac_enabled == 1 || settings.auto_discovery_enabled == 1 {
        return Err(AppError::stt_network(
            "macOS selected PAC or automatic proxy discovery, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
        ));
    }
    if settings.https_enabled == 0 {
        if settings.socks_enabled == 1 {
            return Err(AppError::stt_network(
                "macOS selected a SOCKS system proxy, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
            ));
        }
        return Ok(direct_matcher(no_proxy));
    }

    let host = settings
        .https_host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| {
            AppError::stt_network(
                "macOS has an HTTPS proxy enabled but its host is missing; refusing a direct OpenAI connection.",
            )
        })?;
    let port = settings.https_port.ok_or_else(|| {
        AppError::stt_network(
            "macOS has an HTTPS proxy enabled but its port is missing; refusing a direct OpenAI connection.",
        )
    })?;
    let port = u16::try_from(port).ok().filter(|port| *port != 0).ok_or_else(|| {
        AppError::stt_network(
            "macOS has an HTTPS proxy enabled but its port is invalid; refusing a direct OpenAI connection.",
        )
    })?;
    matcher_for_configured_https_proxy(&authority_with_port(host, port), no_proxy)
}

#[cfg(target_os = "windows")]
fn system_proxy_matcher_without_explicit_https(no_proxy: Option<String>) -> AppResult<Matcher> {
    let settings = windows_registry::CURRENT_USER
        .open("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .map_err(|error| {
            AppError::stt_network(format!(
                "Windows system proxy settings could not be read; refusing a direct OpenAI connection: {error}"
            ))
        })?;
    let enabled = settings.get_u32("ProxyEnable").map_err(|error| {
        AppError::stt_network(format!(
            "Windows ProxyEnable could not be read; refusing a direct OpenAI connection: {error}"
        ))
    })?;
    if enabled == 0 {
        return Ok(direct_matcher(no_proxy.as_deref()));
    }
    if enabled != 1 {
        return Err(AppError::stt_network(format!(
            "Windows reported an invalid ProxyEnable value ({enabled}); refusing a direct OpenAI connection."
        )));
    }
    let proxy_server = settings.get_string("ProxyServer").map_err(|error| {
        AppError::stt_network(format!(
            "Windows has a system proxy enabled but ProxyServer could not be read: {error}"
        ))
    })?;
    let no_proxy = no_proxy.or_else(|| {
        settings
            .get_string("ProxyOverride")
            .ok()
            .map(|value| normalized_no_proxy(&value))
    });
    matcher_for_windows_proxy_server(&proxy_server, no_proxy.as_deref())
}

fn direct_matcher(no_proxy: Option<&str>) -> Matcher {
    let mut builder = Matcher::builder();
    if let Some(no_proxy) = no_proxy {
        builder = builder.no(normalized_no_proxy(no_proxy));
    }
    builder.build()
}

#[cfg(any(target_os = "windows", test))]
fn matcher_for_windows_proxy_server(
    proxy_server: &str,
    no_proxy: Option<&str>,
) -> AppResult<Matcher> {
    let Some(proxy) = windows_https_proxy(proxy_server)? else {
        return Ok(direct_matcher(no_proxy));
    };
    matcher_for_configured_https_proxy(&proxy, no_proxy)
}

#[cfg(any(target_os = "windows", test))]
fn windows_https_proxy(proxy_server: &str) -> AppResult<Option<String>> {
    let proxy_server = proxy_server.trim();
    if proxy_server.is_empty() {
        return Err(AppError::stt_network(
            "Windows has a system proxy enabled but ProxyServer is empty.",
        ));
    }
    let is_protocol_map = proxy_server.split(';').any(|entry| {
        entry
            .split_once('=')
            .is_some_and(|(protocol, _)| !protocol.contains("://"))
    });
    if !is_protocol_map {
        return Ok(Some(proxy_server.to_string()));
    }

    let mut https_proxy = None;
    let mut socks_proxy = None;
    for entry in proxy_server
        .split(';')
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let (protocol, proxy) = entry.split_once('=').ok_or_else(|| {
            AppError::stt_network(format!(
                "Windows ProxyServer contains an invalid protocol entry: {entry}."
            ))
        })?;
        let proxy = proxy.trim();
        if proxy.is_empty() {
            return Err(AppError::stt_network(format!(
                "Windows ProxyServer has an empty {protocol} proxy address."
            )));
        }
        match protocol.trim().to_ascii_lowercase().as_str() {
            "https" => {
                if https_proxy.replace(proxy.to_string()).is_some() {
                    return Err(AppError::stt_network(
                        "Windows ProxyServer contains more than one HTTPS proxy.",
                    ));
                }
            }
            "socks" => socks_proxy = Some(proxy.to_string()),
            _ => {}
        }
    }
    if let Some(proxy) = https_proxy {
        return Ok(Some(proxy));
    }
    if socks_proxy.is_some() {
        return Err(AppError::stt_network(
            "Windows selected a SOCKS system proxy, which is not supported for OpenAI Realtime; use an HTTP CONNECT proxy.",
        ));
    }
    Ok(None)
}

fn connect_with_matcher(request: &Request, matcher: &Matcher) -> AppResult<TcpStream> {
    let target = Target::from_request(request)?;
    let match_uri = https_proxy_match_uri(request.uri())?;
    let deadline = Instant::now() + PROXY_CONNECT_BUDGET;

    let Some(proxy) = matcher.intercept(&match_uri) else {
        return connect_tcp(&target.host, target.port, deadline, "OpenAI Realtime");
    };

    connect_http_proxy(&target, &proxy, deadline)
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
    deadline: Instant,
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
        proxy_host,
        proxy_port,
        deadline,
        "the selected system proxy",
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
    host: &str,
    port: u16,
    deadline: Instant,
    destination: &str,
) -> AppResult<TcpStream> {
    let addresses = (host, port).to_socket_addrs().map_err(|error| {
        AppError::stt_network(format!("Failed to resolve {destination}: {error}"))
    })?;
    let mut last_error = None;
    for address in addresses {
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
