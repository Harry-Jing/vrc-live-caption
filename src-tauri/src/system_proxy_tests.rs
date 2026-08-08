use super::*;
use crate::host_resolver::HostResolver;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::http::Request;

const TEST_API_KEY: &str = "test-openai-api-key-must-not-reach-proxy";

#[test]
fn no_proxy_match_connects_directly() -> AppResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::state(format!("Failed to bind test listener: {error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::state(format!("Failed to read test address: {error}")))?;
    let accept = thread::spawn(move || listener.accept().map(|_| ()));
    let request = test_request(&format!("wss://{address}/realtime"))?;

    let stream = connect_with_matcher(&request, &Matcher::builder().build())?;
    drop(stream);
    join_server(accept)?;
    Ok(())
}

#[test]
fn https_proxy_match_uses_connect_without_openai_authorization() -> AppResult<()> {
    let (proxy_uri, server) =
        spawn_proxy(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: test\r\n\r\n".to_vec())?;
    let authenticated_proxy_uri =
        proxy_uri.replacen("http://", "http://proxy-user:proxy-password@", 1);
    let matcher = Matcher::builder().https(authenticated_proxy_uri).build();
    let request = test_request("wss://api.openai.com/v1/realtime?intent=transcription")?;

    let stream = connect_with_matcher(&request, &matcher)?;
    drop(stream);
    let received = join_server(server)?;
    let received = String::from_utf8(received)
        .map_err(|error| AppError::state(format!("Proxy request was not UTF-8: {error}")))?;

    assert!(received.starts_with("CONNECT api.openai.com:443 HTTP/1.1\r\n"));
    assert!(received.contains("Host: api.openai.com:443\r\n"));
    assert!(
        received.contains("Proxy-Authorization: Basic cHJveHktdXNlcjpwcm94eS1wYXNzd29yZA==\r\n")
    );
    assert!(!received.contains(TEST_API_KEY));
    assert!(!received.contains("Authorization: Bearer"));
    Ok(())
}

#[test]
fn any_successful_2xx_connect_status_opens_the_tunnel() -> AppResult<()> {
    let (proxy_uri, server) = spawn_proxy(b"HTTP/1.1 204 No Content\r\n\r\n".to_vec())?;
    let matcher = Matcher::builder().https(proxy_uri).build();
    let request = test_request("wss://api.openai.com/v1/realtime")?;

    let stream = connect_with_matcher(&request, &matcher)?;
    drop(stream);
    let _ = join_server(server)?;
    Ok(())
}

#[test]
fn proxy_authentication_required_is_explicit() -> AppResult<()> {
    let (proxy_uri, server) =
        spawn_proxy(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n".to_vec())?;
    let matcher = Matcher::builder().https(proxy_uri).build();
    let request = test_request("wss://api.openai.com/v1/realtime")?;

    let error = connect_with_matcher(&request, &matcher)
        .err()
        .ok_or_else(|| AppError::state("A 407 proxy response unexpectedly succeeded."))?;
    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("407"));
    assert!(error.to_string().contains("authentication"));
    let _ = join_server(server)?;
    Ok(())
}

#[test]
fn other_non_success_proxy_status_is_rejected() -> AppResult<()> {
    let (proxy_uri, server) = spawn_proxy(b"HTTP/1.1 502 Bad Gateway\r\n\r\n".to_vec())?;
    let matcher = Matcher::builder().https(proxy_uri).build();
    let request = test_request("wss://api.openai.com/v1/realtime")?;

    let error = connect_with_matcher(&request, &matcher)
        .err()
        .ok_or_else(|| AppError::state("A 502 proxy response unexpectedly succeeded."))?;
    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("502"));
    let _ = join_server(server)?;
    Ok(())
}

#[test]
fn oversized_proxy_response_header_is_rejected() -> AppResult<()> {
    let mut response = b"HTTP/1.1 200 Connection Established\r\nX-Fill: ".to_vec();
    response.resize(MAX_CONNECT_RESPONSE_HEADER_BYTES, b'a');
    let (proxy_uri, server) = spawn_proxy(response)?;
    let matcher = Matcher::builder().https(proxy_uri).build();
    let request = test_request("wss://api.openai.com/v1/realtime")?;

    let error = connect_with_matcher(&request, &matcher)
        .err()
        .ok_or_else(|| AppError::state("An oversized proxy response unexpectedly succeeded."))?;
    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("16 KiB"));
    let _ = join_server(server)?;
    Ok(())
}

#[test]
fn unsupported_selected_proxy_scheme_is_rejected() -> AppResult<()> {
    let matcher = Matcher::builder().https("socks5://127.0.0.1:1080").build();
    let request = test_request("wss://api.openai.com/v1/realtime")?;

    let error = connect_with_matcher(&request, &matcher)
        .err()
        .ok_or_else(|| AppError::state("A SOCKS proxy unexpectedly succeeded."))?;
    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("socks5"));
    assert!(error.to_string().contains("not supported"));
    Ok(())
}

#[test]
fn selected_proxy_failure_never_falls_back_to_direct() -> AppResult<()> {
    let closed_proxy = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::state(format!("Failed to reserve proxy port: {error}")))?;
    let closed_proxy_address = closed_proxy
        .local_addr()
        .map_err(|error| AppError::state(format!("Failed to read proxy address: {error}")))?;
    drop(closed_proxy);

    let target = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::state(format!("Failed to bind direct target: {error}")))?;
    target
        .set_nonblocking(true)
        .map_err(|error| AppError::state(format!("Failed to configure direct target: {error}")))?;
    let target_address = target
        .local_addr()
        .map_err(|error| AppError::state(format!("Failed to read target address: {error}")))?;
    let matcher = Matcher::builder()
        .https(format!("http://{closed_proxy_address}"))
        .build();
    let request = test_request(&format!("wss://{target_address}/realtime"))?;

    let error = connect_with_matcher(&request, &matcher)
        .err()
        .ok_or_else(|| AppError::state("A closed selected proxy unexpectedly succeeded."))?;
    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("system proxy"));
    let accept_error = target.accept().err().ok_or_else(|| {
        AppError::state("The connection silently fell back to the direct target.")
    })?;
    assert_eq!(accept_error.kind(), ErrorKind::WouldBlock);
    Ok(())
}

#[test]
fn malformed_configured_proxy_is_invalid_instead_of_direct() -> AppResult<()> {
    let target = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::state(format!("Failed to bind direct target: {error}")))?;
    target
        .set_nonblocking(true)
        .map_err(|error| AppError::state(format!("Failed to configure direct target: {error}")))?;

    let error = matcher_for_configured_https_proxy("http://[invalid", None)
        .err()
        .ok_or_else(|| AppError::state("A malformed configured proxy was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("proxy address is invalid"));
    let accept_error = target
        .accept()
        .err()
        .ok_or_else(|| AppError::state("Malformed proxy configuration connected directly."))?;
    assert_eq!(accept_error.kind(), ErrorKind::WouldBlock);
    Ok(())
}

#[test]
fn proxy_bypass_accepts_windows_whitespace_and_environment_commas() {
    assert_eq!(
        normalized_no_proxy(
            "*.internal.example localhost\tservice.example,environment.example;last.example"
        ),
        "internal.example,localhost,service.example,environment.example,last.example"
    );
}

#[test]
fn direct_hostname_resolution_obeys_the_connection_deadline() -> AppResult<()> {
    let resolver = HostResolver::with_lookup(|_, _| {
        thread::sleep(Duration::from_millis(100));
        Ok(vec![std::net::SocketAddr::from(([127, 0, 0, 1], 9))])
    });
    let request = test_request("wss://blocked.test/realtime")?;
    let started_at = Instant::now();

    let error = connect_with_matcher_until(
        &request,
        &Matcher::builder().build(),
        &resolver,
        started_at + Duration::from_millis(20),
        &|| false,
    )
    .err()
    .ok_or_else(|| AppError::state("A DNS lookup exceeded the OpenAI connection deadline."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("timed out"));
    Ok(())
}

#[test]
fn selected_proxy_hostname_resolution_observes_cancellation() -> AppResult<()> {
    let matcher = Matcher::builder().https("http://proxy.test:8080").build();
    let request = test_request("wss://api.openai.com/realtime")?;

    let error = connect_with_matcher_until(
        &request,
        &matcher,
        &HostResolver::default(),
        Instant::now() + Duration::from_secs(1),
        &|| true,
    )
    .err()
    .ok_or_else(|| AppError::state("A cancelled proxy hostname unexpectedly connected."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("cancelled"));
    Ok(())
}

fn test_request(uri: &str) -> AppResult<Request<()>> {
    Request::builder()
        .uri(uri)
        .header("Authorization", format!("Bearer {TEST_API_KEY}"))
        .body(())
        .map_err(|error| AppError::state(format!("Failed to build test request: {error}")))
}

fn spawn_proxy(
    response: Vec<u8>,
) -> AppResult<(String, thread::JoinHandle<std::io::Result<Vec<u8>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::state(format!("Failed to bind test proxy: {error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::state(format!("Failed to read proxy address: {error}")))?;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        let request = read_header(&mut stream)?;
        stream.write_all(&response)?;
        Ok(request)
    });
    Ok((format!("http://{address}"), server))
}

fn read_header(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        let count = stream.read(&mut byte)?;
        if count == 0 {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "client closed before the request header completed",
            ));
        }
        request.push(byte[0]);
    }
    Ok(request)
}

fn join_server<T>(handle: thread::JoinHandle<std::io::Result<T>>) -> AppResult<T> {
    handle
        .join()
        .map_err(|_| AppError::state("Test proxy thread panicked."))?
        .map_err(|error| AppError::state(format!("Test proxy failed: {error}")))
}
