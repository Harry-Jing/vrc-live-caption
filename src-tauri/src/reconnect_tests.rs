use super::*;
use crate::error::{AppError, RetryDisposition};
use std::time::Duration;

#[test]
fn generated_jitter_stays_within_the_inclusive_policy_range() {
    for _ in 0..64 {
        let jitter_percent = reconnect_jitter_percent();
        assert!((80..=120).contains(&jitter_percent));
    }
}

#[test]
fn terminal_failures_never_enter_the_reconnect_loop() {
    let mut supervisor = ReconnectSupervisor::default();

    assert_eq!(supervisor.begin_connection_attempt(), 1);
    assert_eq!(
        supervisor.on_failure(
            &AppError::stt_network_terminal("Invalid proxy configuration."),
            None,
            100,
        ),
        ReconnectDecision::Terminal
    );
}

#[test]
fn transient_failures_back_off_with_a_cap_and_monotonic_connection_epochs() {
    let mut supervisor = ReconnectSupervisor::default();
    let transient = AppError::stt_network_retryable("Connection reset.");

    assert_eq!(transient.retry_disposition(), RetryDisposition::Retryable);
    assert_eq!(supervisor.begin_connection_attempt(), 1);
    assert_eq!(
        supervisor.on_failure(&transient, None, 100),
        ReconnectDecision::Retry {
            attempt: 1,
            delay: Duration::from_millis(500),
        }
    );
    assert_eq!(supervisor.begin_connection_attempt(), 2);
    assert_eq!(
        supervisor.on_failure(&transient, None, 100),
        ReconnectDecision::Retry {
            attempt: 2,
            delay: Duration::from_secs(1),
        }
    );

    for _ in 0..10 {
        let _ = supervisor.on_failure(&transient, None, 100);
    }
    assert_eq!(
        supervisor.on_failure(&transient, None, 100),
        ReconnectDecision::Retry {
            attempt: 13,
            delay: Duration::from_secs(30),
        }
    );
}

#[test]
fn a_flapping_connection_keeps_the_accumulated_backoff() {
    let mut supervisor = ReconnectSupervisor::default();
    let transient = AppError::stt_network_retryable("Connection reset.");

    assert_eq!(supervisor.begin_connection_attempt(), 1);
    let _ = supervisor.on_failure(&transient, None, 100);
    assert_eq!(supervisor.begin_connection_attempt(), 2);

    assert_eq!(
        supervisor.on_failure(&transient, Some(Duration::from_secs(1)), 80),
        ReconnectDecision::Retry {
            attempt: 2,
            delay: Duration::from_millis(800),
        }
    );
    assert_eq!(supervisor.begin_connection_attempt(), 3);
}

#[test]
fn a_stable_connection_resets_only_the_backoff_not_the_epoch() {
    let mut supervisor = ReconnectSupervisor::default();
    let transient = AppError::stt_network_retryable("Connection reset.");

    assert_eq!(supervisor.begin_connection_attempt(), 1);
    let _ = supervisor.on_failure(&transient, None, 100);
    assert_eq!(supervisor.begin_connection_attempt(), 2);

    assert_eq!(
        supervisor.on_failure(&transient, Some(BACKOFF_RESET_AFTER_STABLE_CONNECTION), 80,),
        ReconnectDecision::Retry {
            attempt: 1,
            delay: Duration::from_millis(400),
        }
    );
    assert_eq!(supervisor.begin_connection_attempt(), 3);
}

#[test]
fn failed_startup_attempts_do_not_label_the_first_running_capture_as_recovered() {
    let mut supervisor = ReconnectSupervisor::default();
    let transient = AppError::stt_network_retryable("Handshake reset.");

    assert_eq!(supervisor.begin_connection_attempt(), 1);
    let _ = supervisor.on_failure(&transient, None, 100);
    assert_eq!(supervisor.begin_connection_attempt(), 2);
    assert!(!supervisor.is_recovery());

    supervisor.mark_running();
    assert!(supervisor.is_recovery());
}
