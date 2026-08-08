use super::*;
use crate::error::{AppResult, ProviderFailureClass, RetryDisposition};
use std::io::{self, ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tungstenite::error::ProtocolError;
use tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

#[test]
fn rustls_crypto_provider_is_available_for_websocket_tls() {
    let _provider = rustls::crypto::ring::default_provider();
    let _builder = rustls::ClientConfig::builder();
}

#[test]
fn handshake_request_uses_transcription_intent_without_session_model() -> AppResult<()> {
    let api_key = SecretString::from("test-api-key".to_string());
    let request = openai_websocket_request(&api_key)?;

    assert_eq!(
        request.uri().to_string(),
        "wss://api.openai.com/v1/realtime?intent=transcription"
    );
    let authorization = request
        .headers()
        .get("Authorization")
        .ok_or_else(|| AppError::state("WebSocket request did not include Authorization."))?
        .to_str()
        .map_err(|error| AppError::state(format!("Invalid test Authorization header: {error}")))?;
    assert_eq!(authorization, "Bearer test-api-key");
    assert!(!request.uri().to_string().contains("model="));
    assert!(request.headers().get("OpenAI-Beta").is_none());
    Ok(())
}

#[test]
fn empty_api_key_is_rejected_before_any_network_connection() {
    let api_key = SecretString::from("   ".to_string());
    assert!(openai_websocket_request(&api_key).is_err());
}

#[test]
fn handshake_http_statuses_produce_actionable_error_categories() {
    let auth_error = map_handshake_http_status(401, None);
    assert_eq!(auth_error.code(), "config.secret_failed");
    assert!(auth_error.to_string().contains("API key or project access"));

    let rate_error = map_handshake_http_status(429, None);
    assert_eq!(
        rate_error.provider_failure_class(),
        Some(ProviderFailureClass::RateLimited)
    );
    assert_eq!(rate_error.retry_disposition(), RetryDisposition::Retryable);
    assert!(rate_error.to_string().contains("rate-limited"));

    let provider_error = map_handshake_http_status(503, None);
    assert_eq!(
        provider_error.provider_failure_class(),
        Some(ProviderFailureClass::ServiceUnavailable)
    );
    assert_eq!(
        provider_error.retry_disposition(),
        RetryDisposition::Retryable
    );
    assert!(
        provider_error
            .to_string()
            .contains("temporarily unavailable")
    );
}

#[test]
fn handshake_429_uses_structured_quota_metadata_without_exposing_provider_text() {
    let canary = "provider-handshake-message-canary";
    let body = format!(
        r#"{{"error":{{"type":"insufficient_quota","code":"insufficient_quota","message":"{canary}","param":"secret-param"}}}}"#
    );

    let error = map_handshake_http_status(429, Some(body.as_bytes()));
    let observable = format!("{error:?}\n{error}");

    assert_eq!(
        error.provider_failure_class(),
        Some(ProviderFailureClass::UsageLimit)
    );
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert_eq!(error.code(), "stt.provider_usage_limit");
    assert!(!observable.contains(canary));
    assert!(!observable.contains("secret-param"));
}

#[test]
fn only_transient_socket_failures_are_retryable() {
    for kind in [
        ErrorKind::ConnectionReset,
        ErrorKind::NetworkUnreachable,
        ErrorKind::HostUnreachable,
    ] {
        let error = map_socket_error("read failed", WebSocketError::Io(io::Error::from(kind)));
        assert_eq!(error.retry_disposition(), RetryDisposition::Retryable);
    }

    let invalid_tls = map_socket_error(
        "TLS failed",
        WebSocketError::Tls(tungstenite::error::TlsError::InvalidDnsName),
    );
    assert_eq!(invalid_tls.retry_disposition(), RetryDisposition::Terminal);
}

#[test]
fn abrupt_websocket_reset_and_would_block_are_retryable_without_widening_protocol_errors() {
    let reset = map_socket_error(
        "read failed",
        WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake),
    );
    let would_block = map_socket_error(
        "write failed",
        WebSocketError::Io(io::Error::from(ErrorKind::WouldBlock)),
    );
    let invalid_frame = map_socket_error(
        "read failed",
        WebSocketError::Protocol(ProtocolError::InvalidOpcode(3)),
    );

    assert_eq!(reset.retry_disposition(), RetryDisposition::Retryable);
    assert_eq!(would_block.retry_disposition(), RetryDisposition::Retryable);
    assert_eq!(
        invalid_frame.retry_disposition(),
        RetryDisposition::Terminal
    );
}

#[test]
fn websocket_close_codes_choose_retry_policy_without_exposing_the_reason() {
    let canary = "provider-close-reason-canary";
    for code in [
        CloseCode::Normal,
        CloseCode::Away,
        CloseCode::Error,
        CloseCode::Restart,
        CloseCode::Again,
    ] {
        let frame = CloseFrame {
            code,
            reason: canary.into(),
        };
        let error = map_close_frame(Some(&frame));
        assert_eq!(error.retry_disposition(), RetryDisposition::Retryable);
        assert!(!format!("{error:?}\n{error}").contains(canary));
    }

    for code in [
        CloseCode::Protocol,
        CloseCode::Unsupported,
        CloseCode::Invalid,
        CloseCode::Policy,
        CloseCode::Size,
        CloseCode::Extension,
    ] {
        let frame = CloseFrame {
            code,
            reason: canary.into(),
        };
        let error = map_close_frame(Some(&frame));
        assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
        assert!(!format!("{error:?}\n{error}").contains(canary));
    }

    assert_eq!(
        map_close_frame(None).retry_disposition(),
        RetryDisposition::Retryable
    );
}

#[test]
fn tls_handshake_observes_cancellation_after_client_hello() -> AppResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::state(format!("Failed to bind test TLS peer: {error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::state(format!("Failed to read test TLS address: {error}")))?;
    let handshake_started = Arc::new(AtomicBool::new(false));
    let server_handshake_started = Arc::clone(&handshake_started);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        let mut first_byte = [0_u8; 1];
        if stream.read(&mut first_byte)? == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "client closed before sending a TLS ClientHello",
            ));
        }
        server_handshake_started.store(true, Ordering::SeqCst);
        let mut remainder = Vec::new();
        stream.read_to_end(&mut remainder)?;
        Ok(())
    });
    let tcp = TcpStream::connect(address).map_err(|error| {
        AppError::state(format!("Failed to connect to the test TLS peer: {error}"))
    })?;
    let request = openai_websocket_request(&SecretString::from("test-api-key".to_string()))?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let timer_cancelled = Arc::clone(&cancelled);
    let timer = thread::spawn(move || {
        let wait_started_at = Instant::now();
        while !handshake_started.load(Ordering::SeqCst)
            && wait_started_at.elapsed() < Duration::from_secs(2)
        {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(75));
        let cancelled_at = Instant::now();
        timer_cancelled.store(true, Ordering::SeqCst);
        cancelled_at
    });
    let started_at = Instant::now();

    let result = open_websocket_until(
        request,
        tcp,
        WebSocketConfig::default(),
        started_at + HANDSHAKE_IO_TIMEOUT,
        &|| cancelled.load(Ordering::SeqCst),
    );
    let cancelled_at = timer
        .join()
        .map_err(|_| AppError::state("TLS cancellation timer thread panicked."))?;
    let error = result
        .err()
        .ok_or_else(|| AppError::state("A cancelled TLS handshake unexpectedly succeeded."))?;
    server
        .join()
        .map_err(|_| AppError::state("Test TLS peer thread panicked."))?
        .map_err(|error| AppError::state(format!("Test TLS peer failed: {error}")))?;

    assert!(error.to_string().contains("cancelled"));
    assert!(cancelled_at.elapsed() < Duration::from_secs(1));
    Ok(())
}
