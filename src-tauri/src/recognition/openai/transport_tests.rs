use super::super::attempt::RecognitionAttempt;
use super::super::realtime::{OpenAiRealtimeAttempt, OpenAiRealtimeAttemptContext};
use super::super::{OpenAiRecognitionAttemptFactory, OpenAiRecognitionDriver};
use super::*;
use crate::error::{AppResult, ProviderFailureClass, RetryDisposition};
use crate::recognition::{
    OwnedRecognitionAudioFrame, RecognitionGenerationScope, RecognitionModule, RecognitionSignal,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use tungstenite::error::ProtocolError;
use tungstenite::protocol::{CloseFrame, Role, frame::coding::CloseCode};

const TEST_TLS_ROOT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIBgDCCASegAwIBAgIUPHDUu9WL36yvTmFeNFZVe/qhClcwCgYIKoZIzj0EAwIw
HTEbMBkGA1UEAwwSUnVzdGxzIFJvYnVzdCBSb290MCAXDTc1MDEwMTAwMDAwMFoY
DzQwOTYwMTAxMDAwMDAwWjAdMRswGQYDVQQDDBJSdXN0bHMgUm9idXN0IFJvb3Qw
WTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAASW/VkDFs5iGDQvH8jaXYT4jMx66jo+
5CWKyMt4OlTDdBfKfnmQ9LYeK/PsYfJ8wVizuSlPzXi9je8SnyYejGP3o0MwQTAP
BgNVHQ8BAf8EBQMDB4QAMB0GA1UdDgQWBBRqY/oMENJbNo7y39iL6GW3tDs0rzAP
BgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0cAMEQCIEUbrmSUjANju9nNpFop
PAl9Wh8tBxI5IY+BPh466+aUAiA1/9+prypt6s3Doo0GDsnoFGJi1UBivUg1qdik
cy4eNw==
-----END CERTIFICATE-----"#;

const TEST_TLS_CHAIN_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIBszCCAVmgAwIBAgIUUg3keFcU1xXWK8BNVb1KynPulV8wCgYIKoZIzj0EAwIw
JjEkMCIGA1UEAwwbUnVzdGxzIFJvYnVzdCBSb290IC0gUnVuZyAyMCAXDTc1MDEw
MTAwMDAwMFoYDzQwOTYwMTAxMDAwMDAwWjAhMR8wHQYDVQQDDBZyY2dlbiBzZWxm
IHNpZ25lZCBjZXJ0MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEud6w4gtZ0xbw
J3E69SSMy5TZfdIifl9L5ZY+hgEe4UiUsBWS32f6Y5NR5Jo8FO1f6o13b3+FvVHR
EHCGdvppL6NoMGYwFQYDVR0RBA4wDIIKZm9vYmFyLmNvbTAdBgNVHSUEFjAUBggr
BgEFBQcDAQYIKwYBBQUHAwIwHQYDVR0OBBYEFELvxbj5tD75n4pYFvJyr+c8qVEi
MA8GA1UdEwEB/wQFMAMBAQAwCgYIKoZIzj0EAwIDSAAwRQIhALxSSdUsrRFnwNMu
/doBqI8i8u5HdohVAheFTDwObkOMAiASSjULUtkWSD15u/7Sr01Wm9J1MpqW1pob
BVqU3CNRlA==
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIIBiTCCATCgAwIBAgIUHWiVYIvMMWoZEFYvSz46COf2FqowCgYIKoZIzj0EAwIw
HTEbMBkGA1UEAwwSUnVzdGxzIFJvYnVzdCBSb290MCAXDTc1MDEwMTAwMDAwMFoY
DzQwOTYwMTAxMDAwMDAwWjAmMSQwIgYDVQQDDBtSdXN0bHMgUm9idXN0IFJvb3Qg
LSBSdW5nIDIwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAATAOCcBD7dXjmAZ3te5
D47cCJ9ec93PWv7BKYIL826CJsKfXQOGrBTthLm77hXLhHu6uv8E5QXNLZpfowLQ
Do1ao0MwQTAPBgNVHQ8BAf8EBQMDB4QAMB0GA1UdDgQWBBRdza76r11Ok9vRmlg6
Nn/wL/N+jTAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0cAMEQCIFmZrXeK
hnfkahocvkhhNT3cDv1LWf6WBoFaCiBwZXFPAiARaKRiSCMG7PCHmSqFe82TBVmL
odHGogAVax1Dh/aYAA==
-----END CERTIFICATE-----"#;

const TEST_TLS_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgTbAQpfjAT46fgF4B
mP15n37woNG5ZNJmwcqsred/7tmhRANCAAS53rDiC1nTFvAncTr1JIzLlNl90iJ+
X0vllj6GAR7hSJSwFZLfZ/pjk1HkmjwU7V/qjXdvf4W9UdEQcIZ2+mkv
-----END PRIVATE KEY-----"#;

#[test]
fn rustls_crypto_provider_is_available_for_websocket_tls() {
    let _provider = rustls::crypto::ring::default_provider();
    let _builder = rustls::ClientConfig::builder();
}

fn test_tls_configs() -> AppResult<(Arc<ClientConfig>, Arc<ServerConfig>)> {
    let root = CertificateDer::from_pem_slice(TEST_TLS_ROOT_PEM.as_bytes()).map_err(|error| {
        AppError::state(format!(
            "Failed to parse the test TLS root certificate: {error}"
        ))
    })?;
    let mut roots = RootCertStore::empty();
    roots.add(root).map_err(|error| {
        AppError::state(format!(
            "Failed to trust the test TLS root certificate: {error}"
        ))
    })?;
    let client = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let chain = CertificateDer::pem_slice_iter(TEST_TLS_CHAIN_PEM.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            AppError::state(format!(
                "Failed to parse the test TLS certificate chain: {error}"
            ))
        })?;
    let private_key =
        PrivateKeyDer::from_pem_slice(TEST_TLS_PRIVATE_KEY_PEM.as_bytes()).map_err(|error| {
            AppError::state(format!("Failed to parse the test TLS private key: {error}"))
        })?;
    let mut server = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, private_key)
        .map_err(|error| {
            AppError::state(format!("Failed to configure the test TLS server: {error}"))
        })?;
    // Readiness probes must observe only the application record owned by each test.
    server.send_tls13_tickets = 0;

    Ok((Arc::new(client), Arc::new(server)))
}

fn server_websocket_frame(opcode: u8, payload: &[u8]) -> AppResult<Vec<u8>> {
    let payload_len = u8::try_from(payload.len()).map_err(|_| {
        AppError::state("Test WebSocket control or text payload exceeded one-byte framing.")
    })?;
    if payload_len >= 126 {
        return Err(AppError::state(
            "Test WebSocket helper only supports payloads shorter than 126 bytes.",
        ));
    }
    let mut frame = Vec::with_capacity(payload.len() + 2);
    frame.extend_from_slice(&[0x80 | opcode, payload_len]);
    frame.extend_from_slice(payload);
    Ok(frame)
}

struct PlainWebSocketHarness {
    transport: OpenAiWebSocketTransport,
    peer: TcpStream,
}

impl PlainWebSocketHarness {
    fn connect(config: WebSocketConfig) -> AppResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
            AppError::state(format!("Failed to bind test WebSocket peer: {error}"))
        })?;
        let address = listener.local_addr().map_err(|error| {
            AppError::state(format!("Failed to read test WebSocket address: {error}"))
        })?;
        let client = TcpStream::connect(address).map_err(|error| {
            AppError::state(format!("Failed to connect to test WebSocket peer: {error}"))
        })?;
        let (peer, _) = listener.accept().map_err(|error| {
            AppError::state(format!("Failed to accept test WebSocket peer: {error}"))
        })?;
        client.set_nonblocking(true).map_err(|error| {
            AppError::state(format!(
                "Failed to configure test socket nonblocking: {error}"
            ))
        })?;

        let socket =
            WebSocket::from_raw_socket(MaybeTlsStream::Plain(client), Role::Client, Some(config));
        Ok(Self {
            transport: OpenAiWebSocketTransport {
                socket,
                tls_pump: None,
                state: OpenAiWebSocketState::Open,
            },
            peer,
        })
    }
}

fn plain_server_websocket(stream: TcpStream) -> OpenAiSocket {
    WebSocket::from_raw_socket(
        MaybeTlsStream::Plain(stream),
        Role::Server,
        Some(WebSocketConfig::default()),
    )
}

fn backpressure_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .write_buffer_size(64 * 1024)
        .max_write_buffer_size(32 * 1024 * 1024)
}

fn plain_client_stream(transport: &mut OpenAiWebSocketTransport) -> AppResult<&mut TcpStream> {
    if transport.tls_pump.is_some() {
        return Err(AppError::state(
            "Test expected a plain WebSocket transport without a TLS owner.",
        ));
    }
    match transport.socket.get_mut() {
        MaybeTlsStream::Plain(client) => Ok(client),
        _ => Err(AppError::state(
            "Test transport was unexpectedly encrypted.",
        )),
    }
}

fn wait_for_plain_client_readable(
    transport: &OpenAiWebSocketTransport,
    minimum_bytes: usize,
    deadline: Instant,
) -> AppResult<()> {
    if transport.tls_pump.is_some() {
        return Err(AppError::state(
            "Test expected a plain WebSocket transport without a TLS owner.",
        ));
    }
    let client = match transport.socket.get_ref() {
        MaybeTlsStream::Plain(client) => client,
        _ => {
            return Err(AppError::state(
                "Test transport was unexpectedly encrypted.",
            ));
        }
    };
    wait_for_socket_readable(client, minimum_bytes, deadline)
}

fn wait_for_socket_readable(
    socket: &TcpStream,
    minimum_bytes: usize,
    deadline: Instant,
) -> AppResult<()> {
    let mut available = [0_u8; 1_024];
    loop {
        match socket.peek(&mut available) {
            Ok(0) => {
                return Err(AppError::state(
                    "Test socket closed before it became readable.",
                ));
            }
            Ok(available_bytes) if available_bytes >= minimum_bytes => return Ok(()),
            Ok(_) => {
                if Instant::now() >= deadline {
                    return Err(AppError::state(format!(
                        "Test socket did not expose {minimum_bytes} readable byte(s) before the deadline."
                    )));
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                if Instant::now() >= deadline {
                    return Err(AppError::state(
                        "Test socket did not become readable before the deadline.",
                    ));
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                return Err(AppError::state(format!(
                    "Failed while waiting for the test socket: {error}"
                )));
            }
        }
    }
}

fn fill_plain_client_send_buffer(transport: &mut OpenAiWebSocketTransport) -> AppResult<()> {
    let client = plain_client_stream(transport)?;
    let filler = [0_u8; 16 * 1024];
    let mut filled_bytes = 0_usize;
    loop {
        match client.write(&filler) {
            Ok(0) => {
                return Err(AppError::state(
                    "Test socket stopped accepting bytes before it reported backpressure.",
                ));
            }
            Ok(written) => {
                filled_bytes = filled_bytes.saturating_add(written);
                if filled_bytes > 64 * 1024 * 1024 {
                    return Err(AppError::state(
                        "Test socket did not report backpressure within 64 MiB.",
                    ));
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(error) => {
                return Err(AppError::state(format!(
                    "Failed while filling the test socket: {error}"
                )));
            }
        }
    }
}

struct LocalTlsHarness {
    transport: OpenAiWebSocketTransport,
    client_probe: TcpStream,
    peer_connection: ServerConnection,
    peer_stream: TcpStream,
}

impl LocalTlsHarness {
    fn connect(config: WebSocketConfig, io_timeout: Duration) -> AppResult<Self> {
        let (client_config, server_config) = test_tls_configs()?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| AppError::state(format!("Failed to bind test TLS peer: {error}")))?;
        let address = listener.local_addr().map_err(|error| {
            AppError::state(format!("Failed to read test TLS address: {error}"))
        })?;
        let mut client = TcpStream::connect(address).map_err(|error| {
            AppError::state(format!("Failed to connect to test TLS peer: {error}"))
        })?;
        let (mut peer_stream, _) = listener
            .accept()
            .map_err(|error| AppError::state(format!("Failed to accept test TLS peer: {error}")))?;
        for stream in [&client, &peer_stream] {
            stream
                .set_read_timeout(Some(io_timeout))
                .and_then(|()| stream.set_write_timeout(Some(io_timeout)))
                .map_err(|error| {
                    AppError::state(format!("Failed to configure test TLS timeout: {error}"))
                })?;
        }

        let peer = thread::spawn(move || -> AppResult<(ServerConnection, TcpStream)> {
            let mut connection = ServerConnection::new(server_config).map_err(|error| {
                AppError::state(format!("Failed to construct the test TLS server: {error}"))
            })?;
            while connection.is_handshaking() {
                connection.complete_io(&mut peer_stream).map_err(|error| {
                    AppError::state(format!("Test TLS server handshake failed: {error}"))
                })?;
            }
            Ok((connection, peer_stream))
        });

        let server_name = ServerName::try_from("foobar.com")
            .map_err(|error| AppError::state(format!("Invalid test TLS server name: {error}")))?;
        let mut client_connection =
            ClientConnection::new(client_config, server_name).map_err(|error| {
                AppError::state(format!("Failed to construct the test TLS client: {error}"))
            })?;
        while client_connection.is_handshaking() {
            client_connection
                .complete_io(&mut client)
                .map_err(|error| {
                    AppError::state(format!("Test TLS client handshake failed: {error}"))
                })?;
        }
        let (peer_connection, peer_stream) = peer
            .join()
            .map_err(|_| AppError::state("Test TLS server thread panicked."))??;
        let client_probe = client.try_clone().map_err(|error| {
            AppError::state(format!("Failed to clone the test TLS client: {error}"))
        })?;

        let socket = WebSocket::from_raw_socket(
            MaybeTlsStream::Rustls(StreamOwned::new(client_connection, client)),
            Role::Client,
            Some(config),
        );
        Ok(Self {
            transport: OpenAiWebSocketTransport::from_established_socket(socket)?,
            client_probe,
            peer_connection,
            peer_stream,
        })
    }
}

struct EstablishedTransportAttemptFactory {
    transport: Option<OpenAiWebSocketTransport>,
}

impl OpenAiRecognitionAttemptFactory for EstablishedTransportAttemptFactory {
    type Attempt = OpenAiRealtimeAttempt<OpenAiWebSocketTransport>;

    fn connect(
        &mut self,
        context: OpenAiRealtimeAttemptContext,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self::Attempt> {
        let transport = self.transport.take().ok_or_else(|| {
            AppError::state("Realtime-rate test attempted to reuse its one transport.")
        })?;
        let mut attempt = OpenAiRealtimeAttempt::connect(
            context,
            OpenAiTranscriptionModel::GptLiveTranscribe,
            vec!["en".to_string()],
            transport,
        )?;
        let deadline = Instant::now() + Duration::from_secs(1);
        while !attempt.is_ready() {
            if is_cancelled() {
                return Err(AppError::state(
                    "Realtime-rate test was cancelled before attempt readiness.",
                ));
            }
            let _events = attempt.drain_events(0)?;
            if Instant::now() >= deadline {
                return Err(AppError::state(
                    "Realtime-rate test attempt did not become ready before its deadline.",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(attempt)
    }
}

fn run_realtime_rate_peer(
    peer: TcpStream,
    received_pcm_samples: Arc<AtomicUsize>,
) -> AppResult<()> {
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| {
            AppError::state(format!(
                "Failed to configure the realtime-rate test peer: {error}"
            ))
        })?;
    let mut socket = plain_server_websocket(peer);
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let event: serde_json::Value =
                    serde_json::from_str(text.as_str()).map_err(|error| {
                        AppError::state(format!(
                            "Realtime-rate test peer received invalid JSON: {error}"
                        ))
                    })?;
                match event.get("type").and_then(serde_json::Value::as_str) {
                    Some("session.update") => socket
                        .send(Message::text(r#"{"type":"session.updated"}"#))
                        .map_err(|error| {
                            AppError::state(format!(
                                "Realtime-rate test peer could not confirm readiness: {error}"
                            ))
                        })?,
                    Some("input_audio_buffer.append") => {
                        let audio = event
                            .get("audio")
                            .and_then(serde_json::Value::as_str)
                            .ok_or_else(|| {
                                AppError::state(
                                    "Realtime-rate audio append omitted its encoded payload.",
                                )
                            })?;
                        let pcm = BASE64_STANDARD.decode(audio).map_err(|error| {
                            AppError::state(format!(
                                "Realtime-rate audio payload was not valid base64: {error}"
                            ))
                        })?;
                        if pcm.len() % 2 != 0 {
                            return Err(AppError::state(
                                "Realtime-rate PCM payload had an incomplete sample.",
                            ));
                        }
                        received_pcm_samples.fetch_add(pcm.len() / 2, Ordering::SeqCst);
                    }
                    Some(_) | None => {}
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(Message::Ping(payload)) => socket.send(Message::Pong(payload)).map_err(|error| {
                AppError::state(format!(
                    "Realtime-rate test peer could not answer Ping: {error}"
                ))
            })?,
            Ok(Message::Pong(_) | Message::Binary(_) | Message::Frame(_)) => {}
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => return Ok(()),
            Err(WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake)) => {
                return Ok(());
            }
            Err(WebSocketError::Io(error))
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::BrokenPipe
                        | ErrorKind::UnexpectedEof
                ) =>
            {
                return Ok(());
            }
            Err(error) => {
                return Err(AppError::state(format!(
                    "Realtime-rate test peer failed: {error}"
                )));
            }
        }
    }
}

// Protocol progress, backpressure, and transport lifecycle.

#[test]
fn realtime_rate_audio_crosses_the_driver_and_transport_without_backpressure() -> AppResult<()> {
    const FRAME_COUNT: u64 = 128;
    const FRAME_SAMPLES: usize = 160;
    const MIN_EXPECTED_PCM_SAMPLES: usize = 30_000;

    let PlainWebSocketHarness { transport, peer } =
        PlainWebSocketHarness::connect(WebSocketConfig::default())?;
    let received_pcm_samples = Arc::new(AtomicUsize::new(0));
    let peer_samples = Arc::clone(&received_pcm_samples);
    let server = thread::spawn(move || run_realtime_rate_peer(peer, peer_samples));
    let driver = OpenAiRecognitionDriver::new(EstablishedTransportAttemptFactory {
        transport: Some(transport),
    });
    let module = RecognitionModule::with_audio_budget(Duration::from_millis(500), 64, driver)?;
    let mut running = module.start(RecognitionGenerationScope {
        generation: 23,
        stream_id: "recognition-23-1".to_string(),
    })?;
    if !matches!(
        running.signals.recv_timeout(Duration::from_secs(1)),
        Ok(RecognitionSignal::Ready { .. })
    ) {
        let _ = running.stop();
        let _ = server.join();
        return Err(AppError::state(
            "Realtime-rate recognition did not become ready.",
        ));
    }

    for sequence in 1..=FRAME_COUNT {
        running
            .try_submit(OwnedRecognitionAudioFrame {
                sequence,
                captured_at_ms: sequence.saturating_mul(10),
                sample_rate_hz: 16_000,
                samples: vec![0.25; FRAME_SAMPLES].into_boxed_slice(),
            })
            .map_err(|error| {
                AppError::state(format!(
                    "Realtime-rate audio hit recognition backpressure at frame {sequence}: {error:?}"
                ))
            })?;
        thread::sleep(Duration::from_millis(10));
    }

    let delivery_deadline = Instant::now() + Duration::from_secs(2);
    while received_pcm_samples.load(Ordering::SeqCst) < MIN_EXPECTED_PCM_SAMPLES
        && Instant::now() < delivery_deadline
    {
        thread::sleep(Duration::from_millis(1));
    }
    let delivered = received_pcm_samples.load(Ordering::SeqCst);
    let stop_result = running.stop();
    let server_result = server
        .join()
        .map_err(|_| AppError::state("Realtime-rate test peer thread panicked."))?;
    stop_result?;
    server_result?;
    if delivered < MIN_EXPECTED_PCM_SAMPLES {
        return Err(AppError::state(format!(
            "Realtime-rate audio delivered only {delivered} PCM samples; expected at least {MIN_EXPECTED_PCM_SAMPLES}."
        )));
    }
    Ok(())
}

#[test]
fn outbound_message_remains_pending_and_does_not_delay_abort_when_socket_would_block()
-> AppResult<()> {
    let PlainWebSocketHarness {
        mut transport,
        peer: _peer,
    } = PlainWebSocketHarness::connect(WebSocketConfig::default())?;
    fill_plain_client_send_buffer(&mut transport)?;

    transport.send_text("queued-on-backpressure".to_string())?;
    let abort_started_at = Instant::now();
    transport.close()?;
    assert!(
        abort_started_at.elapsed() < Duration::from_millis(500),
        "A pending WebSocket write delayed transport abort."
    );
    transport.close()?;
    Ok(())
}

#[test]
fn receive_poll_does_not_restore_blocking_mode_before_a_pending_write() -> AppResult<()> {
    let PlainWebSocketHarness {
        mut transport,
        peer: _peer,
    } = PlainWebSocketHarness::connect(WebSocketConfig::default())?;
    plain_client_stream(&mut transport)?
        .set_write_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| {
            AppError::state(format!("Failed to configure test write timeout: {error}"))
        })?;
    fill_plain_client_send_buffer(&mut transport)?;

    assert_eq!(transport.try_receive_text()?, None);
    transport.send_text("still-nonblocking".to_string())?;
    Ok(())
}

#[test]
fn receive_poll_flushes_the_last_pending_message_once_after_backpressure_clears() -> AppResult<()> {
    const MESSAGE_COUNT: usize = 64;
    const MESSAGE_BYTES: usize = 256 * 1024;

    let PlainWebSocketHarness {
        mut transport,
        peer: server_stream,
    } = PlainWebSocketHarness::connect(backpressure_websocket_config())?;
    server_stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| {
            AppError::state(format!("Failed to configure test peer timeout: {error}"))
        })?;
    let sent_messages = (0..MESSAGE_COUNT)
        .map(|index| {
            let prefix = format!("message-{index:03}:");
            format!("{prefix}{}", "x".repeat(MESSAGE_BYTES - prefix.len()))
        })
        .collect::<Vec<_>>();

    for message in &sent_messages {
        transport.send_text(message.clone())?;
    }

    let (received_sender, received_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let server = thread::spawn(move || -> AppResult<()> {
        let mut socket = plain_server_websocket(server_stream);
        let mut received = Vec::with_capacity(MESSAGE_COUNT);
        while received.len() < MESSAGE_COUNT {
            match socket.read() {
                Ok(Message::Text(text)) => received.push(text.as_str().to_string()),
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                Ok(other) => {
                    return Err(AppError::state(format!(
                        "Test WebSocket peer received an unexpected message: {other:?}"
                    )));
                }
                Err(error) => {
                    return Err(AppError::state(format!(
                        "Test WebSocket peer failed while reading pending messages: {error}"
                    )));
                }
            }
        }
        received_sender
            .send(received)
            .map_err(|_| AppError::state("Could not report received test messages."))?;
        release_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::state("Test WebSocket peer was not released."))?;
        Ok(())
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let received = loop {
        match received_receiver.try_recv() {
            Ok(received) => break received,
            Err(mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                assert_eq!(transport.try_receive_text()?, None);
                thread::sleep(Duration::from_millis(1));
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                let _ = transport.close();
                let server_result = server
                    .join()
                    .map_err(|_| AppError::state("Test WebSocket peer thread panicked."))?;
                return match server_result {
                    Ok(()) => Err(AppError::state(
                        "Pending WebSocket messages did not flush before the deadline.",
                    )),
                    Err(error) => Err(error),
                };
            }
        }
    };
    release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the test WebSocket peer."))?;
    server
        .join()
        .map_err(|_| AppError::state("Test WebSocket peer thread panicked."))??;

    assert_eq!(received, sent_messages);
    Ok(())
}

#[test]
fn peer_close_is_acknowledged_before_the_transport_reports_it() -> AppResult<()> {
    let PlainWebSocketHarness {
        mut transport,
        peer: server_stream,
    } = PlainWebSocketHarness::connect(WebSocketConfig::default())?;
    server_stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|error| {
            AppError::state(format!("Failed to configure test peer timeout: {error}"))
        })?;
    let (close_sent_sender, close_sent_receiver) = mpsc::channel();
    let server = thread::spawn(move || -> AppResult<()> {
        let mut socket = plain_server_websocket(server_stream);
        socket
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "test-peer-close".into(),
            })))
            .map_err(|error| {
                AppError::state(format!("Test WebSocket peer could not send Close: {error}"))
            })?;
        close_sent_sender
            .send(())
            .map_err(|_| AppError::state("Could not report the test peer Close."))?;
        match socket.read() {
            Ok(Message::Close(Some(frame))) if frame.code == CloseCode::Away => Ok(()),
            Ok(message) => Err(AppError::state(format!(
                "Test WebSocket peer expected a Close acknowledgement, got {message:?}."
            ))),
            Err(error) => Err(AppError::state(format!(
                "Test WebSocket peer did not receive a Close acknowledgement: {error}"
            ))),
        }
    });
    close_sent_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Test WebSocket peer did not send Close."))?;
    wait_for_plain_client_readable(&transport, 1, Instant::now() + Duration::from_secs(1))?;

    let close_deadline = Instant::now() + Duration::from_secs(1);
    let error = loop {
        match transport.try_receive_text() {
            Err(error) => break error,
            Ok(None) if Instant::now() < close_deadline => {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(None) => {
                return Err(AppError::state(
                    "Peer Close was not reported as an attempt failure before the deadline.",
                ));
            }
            Ok(Some(message)) => {
                return Err(AppError::state(format!(
                    "Peer Close poll returned unexpected text: {message}"
                )));
            }
        }
    };
    server
        .join()
        .map_err(|_| AppError::state("Test WebSocket peer thread panicked."))??;

    assert_eq!(error.retry_disposition(), RetryDisposition::Retryable);
    Ok(())
}

#[test]
fn backpressured_peer_close_is_reported_only_after_its_acknowledgement_flushes() -> AppResult<()> {
    const MESSAGE_COUNT: usize = 64;
    const MESSAGE_BYTES: usize = 256 * 1024;

    let PlainWebSocketHarness {
        mut transport,
        peer: server_stream,
    } = PlainWebSocketHarness::connect(backpressure_websocket_config())?;
    server_stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| {
            AppError::state(format!("Failed to configure test peer timeout: {error}"))
        })?;
    for index in 0..MESSAGE_COUNT {
        let prefix = format!("queued-before-close-{index:03}:");
        transport.send_text(format!(
            "{prefix}{}",
            "x".repeat(MESSAGE_BYTES - prefix.len())
        ))?;
    }

    let (send_close_sender, send_close_receiver) = mpsc::channel();
    let (close_sent_sender, close_sent_receiver) = mpsc::channel();
    let (allow_read_sender, allow_read_receiver) = mpsc::channel();
    let server = thread::spawn(move || -> AppResult<()> {
        let mut socket = plain_server_websocket(server_stream);
        send_close_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::state("Test peer was not asked to send Close."))?;
        socket
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "backpressured-test-close".into(),
            })))
            .map_err(|error| {
                AppError::state(format!("Test WebSocket peer could not send Close: {error}"))
            })?;
        close_sent_sender
            .send(())
            .map_err(|_| AppError::state("Could not report the test peer Close."))?;
        allow_read_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::state("Test peer was not allowed to drain client data."))?;

        loop {
            match socket.read() {
                Ok(Message::Text(_)) => {}
                Ok(Message::Close(Some(frame))) if frame.code == CloseCode::Away => return Ok(()),
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                Ok(message) => {
                    return Err(AppError::state(format!(
                        "Test WebSocket peer received an unexpected message: {message:?}"
                    )));
                }
                Err(error) => {
                    return Err(AppError::state(format!(
                        "Test WebSocket peer did not receive a Close acknowledgement: {error}"
                    )));
                }
            }
        }
    });
    send_close_sender
        .send(())
        .map_err(|_| AppError::state("Could not ask the test peer to send Close."))?;
    close_sent_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Test WebSocket peer did not send Close."))?;
    wait_for_plain_client_readable(&transport, 1, Instant::now() + Duration::from_secs(1))?;

    let pending_close_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match transport.try_receive_text() {
            Ok(None) if matches!(&transport.state, OpenAiWebSocketState::PeerClosePending(_)) => {
                break;
            }
            Ok(None) if Instant::now() < pending_close_deadline => {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(None) => {
                return Err(AppError::state(
                    "Peer Close did not enter acknowledgement-pending state before the deadline.",
                ));
            }
            Err(error) => {
                return Err(AppError::state(format!(
                    "Peer Close was reported before its acknowledgement could flush: {error}"
                )));
            }
            Ok(Some(message)) => {
                return Err(AppError::state(format!(
                    "Peer Close poll returned unexpected text: {message}"
                )));
            }
        }
    }
    allow_read_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the test WebSocket peer."))?;

    let deadline = Instant::now() + Duration::from_secs(2);
    let close_error = loop {
        match transport.try_receive_text() {
            Err(error) => break error,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(1)),
            Ok(None) => {
                let _ = transport.close();
                return Err(AppError::state(
                    "Backpressured peer Close was not reported before the deadline.",
                ));
            }
            Ok(Some(message)) => {
                return Err(AppError::state(format!(
                    "Peer Close poll returned unexpected text: {message}"
                )));
            }
        }
    };
    server
        .join()
        .map_err(|_| AppError::state("Test WebSocket peer thread panicked."))??;

    assert_eq!(close_error.retry_disposition(), RetryDisposition::Retryable);
    Ok(())
}

#[test]
fn control_frame_flood_yields_between_polls_and_acknowledges_each_ping() -> AppResult<()> {
    const PING_COUNT: usize = MAX_WEBSOCKET_CONTROL_FRAMES_PER_POLL + 1;

    let PlainWebSocketHarness {
        mut transport,
        peer: server_stream,
    } = PlainWebSocketHarness::connect(WebSocketConfig::default())?;
    server_stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|error| {
            AppError::state(format!("Failed to configure test peer timeout: {error}"))
        })?;
    let (messages_sent_sender, messages_sent_receiver) = mpsc::channel();
    let server = thread::spawn(move || -> AppResult<()> {
        let mut server_stream = server_stream;
        let mut inbound = Vec::new();
        for index in 0..PING_COUNT {
            inbound.extend(server_websocket_frame(0x09, &[index as u8])?);
        }
        inbound.extend(server_websocket_frame(0x01, b"after-control-frames")?);
        server_stream.write_all(&inbound).map_err(|error| {
            AppError::state(format!(
                "Test WebSocket peer could not send messages: {error}"
            ))
        })?;
        messages_sent_sender
            .send(inbound.len())
            .map_err(|_| AppError::state("Could not report the test peer messages."))?;

        let mut socket = plain_server_websocket(server_stream);
        let mut pong_payloads = Vec::with_capacity(PING_COUNT);
        while pong_payloads.len() < PING_COUNT {
            match socket.read() {
                Ok(Message::Pong(payload)) => pong_payloads.push(payload.to_vec()),
                Ok(Message::Ping(_) | Message::Frame(_)) => {}
                Ok(message) => {
                    return Err(AppError::state(format!(
                        "Test WebSocket peer received an unexpected message: {message:?}"
                    )));
                }
                Err(error) => {
                    return Err(AppError::state(format!(
                        "Test WebSocket peer did not receive all Pong replies: {error}"
                    )));
                }
            }
        }
        let expected = (0..PING_COUNT)
            .map(|index| vec![index as u8])
            .collect::<Vec<_>>();
        if pong_payloads != expected {
            return Err(AppError::state(format!(
                "Pong replies were reordered or duplicated: {pong_payloads:?}"
            )));
        }
        Ok(())
    });
    let inbound_bytes = messages_sent_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Test WebSocket peer did not send its messages."))?;
    wait_for_plain_client_readable(
        &transport,
        inbound_bytes,
        Instant::now() + Duration::from_secs(1),
    )?;

    assert_eq!(transport.try_receive_text()?, None);
    let deadline = Instant::now() + Duration::from_secs(1);
    let received = loop {
        match transport.try_receive_text()? {
            Some(message) => break message,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(1)),
            None => {
                return Err(AppError::state(
                    "Text after the control-frame flood was not received before the deadline.",
                ));
            }
        }
    };
    server
        .join()
        .map_err(|_| AppError::state("Test WebSocket peer thread panicked."))??;

    assert_eq!(received, "after-control-frames");
    Ok(())
}

#[test]
fn transport_poll_is_nonblocking_and_preserves_partial_frame_state() -> AppResult<()> {
    let PlainWebSocketHarness {
        mut transport,
        peer: mut server,
    } = PlainWebSocketHarness::connect(WebSocketConfig::default())?;

    let started_at = Instant::now();
    assert_eq!(transport.try_receive_text()?, None);
    assert!(
        started_at.elapsed() < Duration::from_millis(500),
        "An idle transport poll waited for the blocking socket read timeout."
    );

    server
        .write_all(&[0x81, 0x02, b'o'])
        .map_err(|error| AppError::state(format!("Failed to send a partial frame: {error}")))?;
    wait_for_plain_client_readable(&transport, 1, Instant::now() + Duration::from_secs(1))?;
    let partial_started_at = Instant::now();
    assert_eq!(transport.try_receive_text()?, None);
    assert!(
        partial_started_at.elapsed() < Duration::from_millis(500),
        "A partial-frame poll waited for the blocking socket read timeout."
    );

    server
        .write_all(b"k")
        .map_err(|error| AppError::state(format!("Failed to complete a test frame: {error}")))?;
    let message_deadline = Instant::now() + Duration::from_secs(1);
    let message = loop {
        if let Some(message) = transport.try_receive_text()? {
            break message;
        }
        if Instant::now() >= message_deadline {
            return Err(AppError::state(
                "Completed test frame did not become readable before the deadline.",
            ));
        }
        thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(message, "ok");
    Ok(())
}

#[test]
fn transport_poll_is_nonblocking_and_preserves_partial_tls_record_state() -> AppResult<()> {
    let LocalTlsHarness {
        mut transport,
        client_probe,
        peer_connection: mut tls,
        peer_stream: mut server_stream,
    } = LocalTlsHarness::connect(WebSocketConfig::default(), Duration::from_secs(2))?;
    let (first_fragment_sender, first_fragment_receiver) = mpsc::channel();
    let (send_remainder_sender, send_remainder_receiver) = mpsc::channel();
    let (message_received_sender, message_received_receiver) = mpsc::channel();
    let server = thread::spawn(move || -> AppResult<()> {
        tls.writer()
            .write_all(&[0x81, 0x02, b'o', b'k'])
            .map_err(|error| {
                AppError::state(format!(
                    "Failed to queue test TLS application data: {error}"
                ))
            })?;
        let mut encrypted_record = Vec::new();
        while tls.wants_write() {
            let written = tls.write_tls(&mut encrypted_record).map_err(|error| {
                AppError::state(format!("Failed to encode the test TLS record: {error}"))
            })?;
            if written == 0 {
                return Err(AppError::state(
                    "Test TLS server stopped encoding a pending record.",
                ));
            }
        }
        let split_at = encrypted_record.len().checked_sub(1).ok_or_else(|| {
            AppError::state("Test TLS server did not encode an application record.")
        })?;
        server_stream
            .write_all(&encrypted_record[..split_at])
            .and_then(|()| server_stream.flush())
            .map_err(|error| {
                AppError::state(format!(
                    "Failed to send the first TLS record fragment: {error}"
                ))
            })?;
        first_fragment_sender
            .send(())
            .map_err(|_| AppError::state("Could not report the first TLS record fragment."))?;
        send_remainder_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::state("Test TLS record remainder was not requested."))?;
        server_stream
            .write_all(&encrypted_record[split_at..])
            .and_then(|()| server_stream.flush())
            .map_err(|error| {
                AppError::state(format!("Failed to finish the test TLS record: {error}"))
            })?;
        message_received_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::state("Test TLS message was not consumed."))?;
        Ok(())
    });
    first_fragment_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Test TLS server did not send its first fragment."))?;
    wait_for_socket_readable(&client_probe, 1, Instant::now() + Duration::from_secs(1))?;

    let partial_started_at = Instant::now();
    assert_eq!(transport.try_receive_text()?, None);
    assert!(
        partial_started_at.elapsed() < Duration::from_millis(500),
        "A partial TLS record blocked the transport poll."
    );
    send_remainder_sender
        .send(())
        .map_err(|_| AppError::state("Could not request the test TLS record remainder."))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let message = loop {
        match transport.try_receive_text()? {
            Some(message) => break message,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(1)),
            None => {
                return Err(AppError::state(
                    "Completed TLS record did not become readable before the deadline.",
                ));
            }
        }
    };
    message_received_sender
        .send(())
        .map_err(|_| AppError::state("Could not report the consumed test TLS message."))?;
    server
        .join()
        .map_err(|_| AppError::state("Test TLS server thread panicked."))??;

    assert_eq!(message, "ok");
    Ok(())
}

#[test]
fn tls_inbound_text_ping_and_close_progress_while_outbound_is_backpressured() -> AppResult<()> {
    const MESSAGE_COUNT: usize = 64;
    const MESSAGE_BYTES: usize = 256 * 1024;
    const INBOUND_TEXT: &str = "inbound-despite-outbound-backpressure";
    const PING_PAYLOAD: &[u8] = b"owner-ping";

    let LocalTlsHarness {
        mut transport,
        client_probe: _,
        peer_connection: mut tls,
        peer_stream: mut server_stream,
    } = LocalTlsHarness::connect(backpressure_websocket_config(), Duration::from_secs(5))?;
    let (send_inbound_sender, send_inbound_receiver) = mpsc::channel();
    let (inbound_sent_sender, inbound_sent_receiver) = mpsc::channel();
    let (allow_client_drain_sender, allow_client_drain_receiver) = mpsc::channel();
    let server = thread::spawn(move || -> AppResult<()> {
        send_inbound_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::state("Test TLS server was not asked to send inbound data."))?;

        let mut websocket_bytes = server_websocket_frame(0x09, PING_PAYLOAD)?;
        websocket_bytes.extend(server_websocket_frame(0x01, INBOUND_TEXT.as_bytes())?);
        websocket_bytes.extend(server_websocket_frame(
            0x08,
            &u16::from(CloseCode::Away).to_be_bytes(),
        )?);
        tls.writer().write_all(&websocket_bytes).map_err(|error| {
            AppError::state(format!(
                "Failed to queue test TLS WebSocket frames: {error}"
            ))
        })?;
        let mut encrypted = Vec::new();
        while tls.wants_write() {
            let written = tls.write_tls(&mut encrypted).map_err(|error| {
                AppError::state(format!(
                    "Failed to encode test TLS WebSocket frames: {error}"
                ))
            })?;
            if written == 0 {
                return Err(AppError::state(
                    "Test TLS server stopped encoding pending WebSocket frames.",
                ));
            }
        }
        server_stream
            .write_all(&encrypted)
            .and_then(|()| server_stream.flush())
            .map_err(|error| {
                AppError::state(format!("Failed to send test TLS WebSocket frames: {error}"))
            })?;
        inbound_sent_sender
            .send(())
            .map_err(|_| AppError::state("Could not report sent test TLS frames."))?;
        allow_client_drain_receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| AppError::state("Test TLS server was not allowed to drain the client."))?;

        let mut socket = WebSocket::from_raw_socket(
            StreamOwned::new(tls, server_stream),
            Role::Server,
            Some(WebSocketConfig::default()),
        );
        let mut pong_count = 0;
        let mut close_count = 0;
        let mut app_message_count = 0;
        while pong_count == 0 || close_count == 0 {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    if app_message_count >= MESSAGE_COUNT {
                        return Err(AppError::state(
                            "Test TLS peer received a duplicate application message.",
                        ));
                    }
                    let expected_prefix =
                        format!("tls-queued-before-inbound-{app_message_count:03}:");
                    if text.len() != MESSAGE_BYTES || !text.as_str().starts_with(&expected_prefix) {
                        return Err(AppError::state(format!(
                            "Test TLS peer received application message {app_message_count} out of order or corrupted."
                        )));
                    }
                    app_message_count += 1;
                }
                Ok(Message::Pong(payload)) if payload.as_ref() == PING_PAYLOAD => pong_count += 1,
                Ok(Message::Close(Some(frame))) if frame.code == CloseCode::Away => {
                    close_count += 1;
                }
                Ok(Message::Ping(_) | Message::Frame(_)) => {}
                Ok(message) => {
                    return Err(AppError::state(format!(
                        "Test TLS peer received an unexpected message: {message:?}"
                    )));
                }
                Err(error) => {
                    return Err(AppError::state(format!(
                        "Test TLS peer did not receive Pong and Close acknowledgement: {error}"
                    )));
                }
            }
        }
        if app_message_count != MESSAGE_COUNT || pong_count != 1 || close_count != 1 {
            return Err(AppError::state(format!(
                "Expected {MESSAGE_COUNT} ordered application messages, one Pong, and one Close acknowledgement; got {app_message_count}, {pong_count}, and {close_count}."
            )));
        }
        Ok(())
    });
    for index in 0..MESSAGE_COUNT {
        let prefix = format!("tls-queued-before-inbound-{index:03}:");
        transport.send_text(format!(
            "{prefix}{}",
            "x".repeat(MESSAGE_BYTES - prefix.len())
        ))?;
    }
    send_inbound_sender
        .send(())
        .map_err(|_| AppError::state("Could not ask the test TLS server to send inbound data."))?;
    inbound_sent_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Test TLS server did not send inbound data."))?;

    let inbound_deadline = Instant::now() + Duration::from_secs(1);
    let mut inbound_text = None;
    while Instant::now() < inbound_deadline {
        match transport.try_receive_text() {
            Ok(Some(message)) => inbound_text = Some(message),
            Ok(None) => {}
            Err(error) => {
                let _ = allow_client_drain_sender.send(());
                let _ = transport.close();
                let _ = server.join();
                return Err(AppError::state(format!(
                    "TLS peer Close was reported before its acknowledgement could drain: {error}"
                )));
            }
        }
        if inbound_text.as_deref() == Some(INBOUND_TEXT)
            && matches!(&transport.state, OpenAiWebSocketState::PeerClosePending(_))
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    if inbound_text.as_deref() != Some(INBOUND_TEXT)
        || !matches!(&transport.state, OpenAiWebSocketState::PeerClosePending(_))
    {
        let _ = allow_client_drain_sender.send(());
        let _ = transport.close();
        let _ = server.join();
        return Err(AppError::state(
            "Inbound TLS Ping, text, and Close did not progress while outbound TLS was backpressured.",
        ));
    }

    allow_client_drain_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the test TLS peer."))?;
    let close_deadline = Instant::now() + Duration::from_secs(5);
    let close_error = loop {
        match transport.try_receive_text() {
            Err(error) => break error,
            Ok(None) if Instant::now() < close_deadline => thread::sleep(Duration::from_millis(1)),
            Ok(None) => {
                return Err(AppError::state(
                    "TLS peer Close acknowledgement did not drain before the deadline.",
                ));
            }
            Ok(Some(message)) => {
                return Err(AppError::state(format!(
                    "Unexpected text after TLS peer Close: {message}"
                )));
            }
        }
    };
    server
        .join()
        .map_err(|_| AppError::state("Test TLS server thread panicked."))??;

    assert_eq!(close_error.retry_disposition(), RetryDisposition::Retryable);
    Ok(())
}

// Request construction, failure mapping, and handshake cancellation.

#[test]
fn handshake_request_uses_transcription_intent_without_a_model_query() -> AppResult<()> {
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
fn exhausted_websocket_write_buffer_reports_explicit_backpressure() {
    let error = map_socket_error(
        "send failed",
        WebSocketError::WriteBufferFull(Box::new(Message::text("unsent-client-event"))),
    );

    assert_eq!(error.code(), "stt.backpressure");
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert!(!error.to_string().contains("unsent-client-event"));
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
    let (client_config, _) = test_tls_configs()?;
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

    let result = open_websocket_until_with_connector(
        request,
        tcp,
        WebSocketConfig::default(),
        started_at + HANDSHAKE_IO_TIMEOUT,
        &|| cancelled.load(Ordering::SeqCst),
        Some(Connector::Rustls(client_config)),
    );
    let error = result
        .err()
        .ok_or_else(|| AppError::state("A cancelled TLS handshake unexpectedly succeeded."))?;
    let cancelled_at = timer
        .join()
        .map_err(|_| AppError::state("TLS cancellation timer thread panicked."))?;
    server
        .join()
        .map_err(|_| AppError::state("Test TLS peer thread panicked."))?
        .map_err(|server_error| {
            AppError::state(format!(
                "Test TLS peer failed: {server_error}; client result: {error}"
            ))
        })?;

    assert!(error.to_string().contains("cancelled"));
    assert!(cancelled_at.elapsed() < Duration::from_secs(1));
    Ok(())
}
