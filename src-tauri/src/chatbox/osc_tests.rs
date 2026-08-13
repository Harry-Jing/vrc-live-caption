use super::super::layout::{PreparedChatboxText, prepare_single_message};
use super::super::pacer::{ChatboxPacer, Clock};
use super::*;
use crate::host_resolver::HostResolver;
use rosc::decoder;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

struct AdvancingClock {
    now: Mutex<Instant>,
    sleeps: Mutex<Vec<Duration>>,
}

impl AdvancingClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    fn total_sleep(&self) -> Duration {
        self.sleeps
            .lock()
            .map(|sleeps| sleeps.iter().copied().sum())
            .unwrap_or_default()
    }
}

impl Clock for AdvancingClock {
    fn now(&self) -> Instant {
        self.now
            .lock()
            .map(|now| *now)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }

    fn sleep(&self, duration: Duration) {
        if let Ok(mut sleeps) = self.sleeps.lock() {
            sleeps.push(duration);
        }
        if let Ok(mut now) = self.now.lock() {
            *now += duration;
        }
    }
}

struct ScriptedOscTransport {
    target: String,
    failures: Mutex<VecDeque<bool>>,
    packets: Mutex<Vec<OscPacket>>,
}

impl ScriptedOscTransport {
    fn new(failures: impl IntoIterator<Item = bool>) -> Self {
        Self {
            target: "scripted".to_string(),
            failures: Mutex::new(failures.into_iter().collect()),
            packets: Mutex::new(Vec::new()),
        }
    }

    fn packets(&self) -> Vec<OscPacket> {
        self.packets
            .lock()
            .map(|packets| packets.clone())
            .unwrap_or_default()
    }
}

impl OscTransport for ScriptedOscTransport {
    fn send_packet(&self, packet: &OscPacket) -> AppResult<usize> {
        if let Ok(mut packets) = self.packets.lock() {
            packets.push(packet.clone());
        }
        let should_fail = self
            .failures
            .lock()
            .ok()
            .and_then(|mut failures| failures.pop_front())
            .unwrap_or(false);
        if should_fail {
            return Err(AppError::osc_send(
                &self.target,
                "Scripted OSC failure.".to_string(),
            ));
        }

        encoder::encode(packet)
            .map(|bytes| bytes.len())
            .map_err(|error| AppError::osc_encode(error.to_string()))
    }

    fn target(&self) -> &str {
        &self.target
    }
}

#[test]
fn chatbox_packets_use_the_vrchat_contract() {
    assert_eq!(
        chatbox_input_packet("test"),
        OscPacket::Message(OscMessage {
            addr: "/chatbox/input".to_string(),
            args: vec![
                OscType::String("test".to_string()),
                OscType::Bool(true),
                OscType::Bool(false),
            ],
        })
    );
    assert_eq!(
        typing_indicator_packet(false),
        OscPacket::Message(OscMessage {
            addr: "/chatbox/typing".to_string(),
            args: vec![OscType::Bool(false)],
        })
    );
}

#[test]
fn prepared_page_is_sent_without_rewriting_whitespace() -> AppResult<()> {
    let transport = Arc::new(ScriptedOscTransport::new([false]));
    let sender = ChatboxOscSender::with_transport(transport.clone());
    let page = prepared_text("first line\n  second  line")?;

    sender.send_text(&page)?;

    assert_eq!(
        transport.packets(),
        vec![OscPacket::Message(OscMessage {
            addr: "/chatbox/input".to_string(),
            args: vec![
                OscType::String("first line\n  second  line".to_string()),
                OscType::Bool(true),
                OscType::Bool(false),
            ],
        })]
    );
    Ok(())
}

#[test]
fn runtime_restart_and_osc_test_share_actual_attempt_history() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());

    for text in ["runtime one", OSC_TEST_MESSAGE, "runtime two"] {
        let sender = ChatboxOscSender::with_transport(Arc::new(ScriptedOscTransport::new([false])));
        let text = prepared_text(text)?;
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("OSC attempt was cancelled."))?
            .attempt(|| sender.send_text(&text))?;
    }

    assert_eq!(clock.total_sleep(), Duration::from_secs(2));
    Ok(())
}

#[test]
fn failed_transport_attempt_still_reserves_the_next_opportunity() -> AppResult<()> {
    let clock = Arc::new(AdvancingClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let failing = ChatboxOscSender::with_transport(Arc::new(ScriptedOscTransport::new([true])));
    let succeeding = ChatboxOscSender::with_transport(Arc::new(ScriptedOscTransport::new([false])));
    let failed_text = prepared_text("failed")?;
    let succeeded_text = prepared_text("succeeded")?;

    assert!(
        pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::runtime("Failed OSC attempt was cancelled."))?
            .attempt(|| failing.send_text(&failed_text))
            .is_err()
    );
    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| AppError::runtime("Follow-up OSC attempt was cancelled."))?
        .attempt(|| succeeding.send_text(&succeeded_text))?;

    assert_eq!(clock.total_sleep(), Duration::from_secs(1));
    Ok(())
}

#[test]
fn udp_transport_sends_exact_text_and_typing_packets() -> AppResult<()> {
    let receiver =
        UdpSocket::bind("127.0.0.1:0").map_err(|error| AppError::osc_bind(error.to_string()))?;
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|error| AppError::osc_bind(error.to_string()))?;
    let port = receiver
        .local_addr()
        .map_err(|error| AppError::osc_bind(error.to_string()))?
        .port();
    let sender = ChatboxOscSender::new(
        &OscConfig {
            host: "127.0.0.1".to_string(),
            port,
            enabled: true,
        },
        &HostResolver::default(),
        &|| false,
    )?;

    let text = prepared_text("exact\n  page")?;
    sender.send_text(&text)?;
    sender.send_typing(false)?;

    assert_eq!(
        receive_packet(&receiver)?,
        chatbox_input_packet("exact\n  page")
    );
    assert_eq!(receive_packet(&receiver)?, typing_indicator_packet(false));
    Ok(())
}

#[test]
fn hostname_resolution_uses_the_injected_resolver() -> AppResult<()> {
    let receiver =
        UdpSocket::bind("127.0.0.1:0").map_err(|error| AppError::osc_bind(error.to_string()))?;
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|error| AppError::osc_bind(error.to_string()))?;
    let port = receiver
        .local_addr()
        .map_err(|error| AppError::osc_bind(error.to_string()))?
        .port();
    let resolver = HostResolver::with_lookup(move |host, requested_port| {
        if host != "vrchat.test" || requested_port != port {
            return Err(std::io::Error::other("Unexpected OSC lookup target."));
        }
        Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
    });
    let sender = ChatboxOscSender::new_until(
        &OscConfig {
            host: "vrchat.test".to_string(),
            port,
            enabled: true,
        },
        &resolver,
        Instant::now() + Duration::from_secs(1),
        &|| false,
    )?;

    let text = prepared_text("resolved target")?;
    sender.send_text(&text)?;

    assert_eq!(
        receive_packet(&receiver)?,
        chatbox_input_packet("resolved target")
    );
    Ok(())
}

#[test]
fn hostname_resolution_deadline_maps_to_an_osc_error() -> AppResult<()> {
    let resolver = HostResolver::with_lookup(|_, _| {
        thread::sleep(Duration::from_millis(100));
        Ok(vec![SocketAddr::from(([127, 0, 0, 1], 9000))])
    });
    let target = OscConfig {
        host: "blocked.test".to_string(),
        port: 9000,
        enabled: true,
    };

    let error = ChatboxOscSender::new_until(
        &target,
        &resolver,
        Instant::now() + Duration::from_millis(20),
        &|| false,
    )
    .err()
    .ok_or_else(|| AppError::state("A timed-out OSC hostname unexpectedly resolved."))?;

    assert_eq!(error.code(), "osc.send_failed");
    assert!(error.to_string().contains("timed out"));
    Ok(())
}

#[test]
fn hostname_resolution_cancellation_maps_to_an_osc_error() -> AppResult<()> {
    let target = OscConfig {
        host: "cancelled.test".to_string(),
        port: 9000,
        enabled: true,
    };

    let error = ChatboxOscSender::new_until(
        &target,
        &HostResolver::default(),
        Instant::now() + Duration::from_secs(1),
        &|| true,
    )
    .err()
    .ok_or_else(|| AppError::state("A cancelled OSC hostname unexpectedly resolved."))?;

    assert_eq!(error.code(), "osc.send_failed");
    assert!(error.to_string().contains("cancelled"));
    Ok(())
}

fn receive_packet(receiver: &UdpSocket) -> AppResult<OscPacket> {
    let mut buffer = [0_u8; 1024];
    let (size, _) = receiver
        .recv_from(&mut buffer)
        .map_err(|error| AppError::osc_send("test receiver", error.to_string()))?;
    let (_, packet) = decoder::decode_udp(&buffer[..size])
        .map_err(|error| AppError::osc_encode(error.to_string()))?;
    Ok(packet)
}

fn prepared_text(text: &str) -> AppResult<PreparedChatboxText> {
    prepare_single_message(text)
        .map_err(|error| {
            AppError::runtime(format!("OSC test text could not be prepared: {error:?}"))
        })?
        .ok_or_else(|| AppError::runtime("OSC test text must not be empty."))
}
