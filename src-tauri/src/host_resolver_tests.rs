use super::*;
use crate::error::{AppError, AppResult};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

const WARMUP_HOST: &str = "resolver-warmup.example";
const TEST_SYNC_TIMEOUT: Duration = Duration::from_secs(2);

fn loopback_result(port: u16) -> Vec<SocketAddr> {
    vec![SocketAddr::from(([127, 0, 0, 1], port))]
}

fn warm_resolver(resolver: &HostResolver) -> AppResult<()> {
    let addresses = resolver
        .resolve_until(WARMUP_HOST, 9, Instant::now() + TEST_SYNC_TIMEOUT, &|| {
            false
        })
        .map_err(|error| AppError::state(format!("Resolver warmup failed: {error:?}")))?;
    if addresses != loopback_result(9) {
        return Err(AppError::state(
            "Resolver warmup returned an unexpected address.",
        ));
    }
    Ok(())
}

#[test]
fn numeric_addresses_bypass_the_worker_lookup() -> AppResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup_calls = Arc::clone(&calls);
    let resolver = HostResolver::with_lookup(move |_, port| {
        lookup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(loopback_result(port))
    });

    let addresses = resolver
        .resolve_until(
            "127.0.0.1",
            9000,
            Instant::now() + Duration::from_secs(1),
            &|| false,
        )
        .map_err(|error| AppError::state(format!("Numeric resolution failed: {error:?}")))?;

    assert_eq!(addresses, loopback_result(9000));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn deadline_returns_while_the_os_lookup_is_still_blocked() -> AppResult<()> {
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let release_receiver = Arc::new(Mutex::new(release_receiver));
    let lookup_release = Arc::clone(&release_receiver);
    let resolver = HostResolver::with_lookup(move |host, port| {
        if host == WARMUP_HOST {
            return Ok(loopback_result(port));
        }
        let _ = started_sender.send(());
        lookup_release
            .lock()
            .map_err(|_| io::Error::other("Test release lock was poisoned."))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("Test lookup was not released before its timeout."))?;
        Ok(loopback_result(port))
    });
    warm_resolver(&resolver)?;

    let caller_resolver = resolver.clone();
    let (result_sender, result_receiver) = mpsc::sync_channel(1);
    let caller = thread::spawn(move || {
        let result = caller_resolver.resolve_until(
            "blocked.example",
            443,
            Instant::now() + Duration::from_millis(500),
            &|| false,
        );
        let _ = result_sender.send(result);
    });

    started_receiver
        .recv_timeout(TEST_SYNC_TIMEOUT)
        .map_err(|_| AppError::state("Resolver worker did not begin its blocking lookup."))?;
    let result = result_receiver.recv_timeout(TEST_SYNC_TIMEOUT);
    let _ = release_sender.send(());
    caller
        .join()
        .map_err(|_| AppError::state("Resolver deadline caller thread panicked."))?;
    let result = result
        .map_err(|_| AppError::state("A blocked lookup did not return before the test timeout."))?;

    assert_eq!(result.err(), Some(HostResolutionError::DeadlineExceeded));
    Ok(())
}

#[test]
fn cancellation_returns_while_the_os_lookup_is_still_blocked() -> AppResult<()> {
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let release_receiver = Arc::new(Mutex::new(release_receiver));
    let lookup_release = Arc::clone(&release_receiver);
    let resolver = HostResolver::with_lookup(move |host, port| {
        if host == WARMUP_HOST {
            return Ok(loopback_result(port));
        }
        let _ = started_sender.send(());
        lookup_release
            .lock()
            .map_err(|_| io::Error::other("Test release lock was poisoned."))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("Test lookup was not released before its timeout."))?;
        Ok(loopback_result(port))
    });
    warm_resolver(&resolver)?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let caller_cancelled = Arc::clone(&cancelled);
    let caller_resolver = resolver.clone();
    let (result_sender, result_receiver) = mpsc::sync_channel(1);
    let caller = thread::spawn(move || {
        let result = caller_resolver.resolve_until(
            "blocked.example",
            443,
            Instant::now() + Duration::from_secs(5),
            &|| caller_cancelled.load(Ordering::SeqCst),
        );
        let _ = result_sender.send(result);
    });

    started_receiver
        .recv_timeout(TEST_SYNC_TIMEOUT)
        .map_err(|_| AppError::state("Resolver worker did not begin its blocking lookup."))?;
    cancelled.store(true, Ordering::SeqCst);
    let result = result_receiver.recv_timeout(TEST_SYNC_TIMEOUT);
    let _ = release_sender.send(());
    caller
        .join()
        .map_err(|_| AppError::state("Resolver cancellation caller thread panicked."))?;
    let result = result.map_err(|_| {
        AppError::state("A cancelled lookup did not return before the test timeout.")
    })?;

    assert_eq!(result.err(), Some(HostResolutionError::Cancelled));
    Ok(())
}

#[test]
fn abandoned_queued_requests_are_skipped_after_the_worker_recovers() -> AppResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup_calls = Arc::clone(&calls);
    let (blocker_started_sender, blocker_started_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let release_receiver = Arc::new(Mutex::new(release_receiver));
    let lookup_release = Arc::clone(&release_receiver);
    let resolver = HostResolver::with_lookup(move |host, port| {
        if host == WARMUP_HOST {
            return Ok(loopback_result(port));
        }
        lookup_calls.fetch_add(1, Ordering::SeqCst);
        if host == "worker-blocker.example" {
            let _ = blocker_started_sender.send(());
            lookup_release
                .lock()
                .map_err(|_| io::Error::other("Test release lock was poisoned."))?
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| {
                    io::Error::other("Test lookup was not released before its timeout.")
                })?;
        }
        Ok(loopback_result(port))
    });
    warm_resolver(&resolver)?;

    let blocker_resolver = resolver.clone();
    let (blocker_result_sender, blocker_result_receiver) = mpsc::sync_channel(1);
    let blocker = thread::spawn(move || {
        let result = blocker_resolver.resolve_until(
            "worker-blocker.example",
            443,
            Instant::now() + Duration::from_secs(5),
            &|| false,
        );
        let _ = blocker_result_sender.send(result);
    });
    blocker_started_receiver
        .recv_timeout(TEST_SYNC_TIMEOUT)
        .map_err(|_| AppError::state("Resolver worker did not begin its blocking lookup."))?;

    let cancellation_checks = AtomicUsize::new(0);
    let cancel_after_enqueue = || cancellation_checks.fetch_add(1, Ordering::SeqCst) > 0;
    assert_eq!(
        resolver
            .resolve_until(
                "abandoned.example",
                443,
                Instant::now() + TEST_SYNC_TIMEOUT,
                &cancel_after_enqueue,
            )
            .err(),
        Some(HostResolutionError::Cancelled)
    );

    release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the resolver worker."))?;
    let blocker_result = blocker_result_receiver
        .recv_timeout(TEST_SYNC_TIMEOUT)
        .map_err(|_| AppError::state("Blocking resolver caller did not finish."))?;
    blocker
        .join()
        .map_err(|_| AppError::state("Blocking resolver caller thread panicked."))?;
    blocker_result
        .map_err(|error| AppError::state(format!("Blocking resolver caller failed: {error:?}")))?;

    let addresses = resolver
        .resolve_until(
            "recovered.example",
            443,
            Instant::now() + TEST_SYNC_TIMEOUT,
            &|| false,
        )
        .map_err(|error| AppError::state(format!("Recovered resolution failed: {error:?}")))?;

    assert_eq!(addresses, loopback_result(443));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn one_blocked_worker_has_a_bounded_request_queue() -> AppResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup_calls = Arc::clone(&calls);
    let (lookup_started_sender, lookup_started_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let release_receiver = Arc::new(Mutex::new(release_receiver));
    let lookup_release = Arc::clone(&release_receiver);
    let resolver = HostResolver::with_lookup(move |host, port| {
        if host == WARMUP_HOST {
            return Ok(loopback_result(port));
        }
        lookup_calls.fetch_add(1, Ordering::SeqCst);
        if host == "worker-blocker.example" {
            let _ = lookup_started_sender.send(());
            lookup_release
                .lock()
                .map_err(|_| io::Error::other("Test release lock was poisoned."))?
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| {
                    io::Error::other("Test lookup was not released before its timeout.")
                })?;
        }
        Ok(loopback_result(port))
    });
    warm_resolver(&resolver)?;

    let blocker_resolver = resolver.clone();
    let (blocker_result_sender, blocker_result_receiver) = mpsc::sync_channel(1);
    let blocker = thread::spawn(move || {
        let result = blocker_resolver.resolve_until(
            "worker-blocker.example",
            443,
            Instant::now() + Duration::from_secs(5),
            &|| false,
        );
        let _ = blocker_result_sender.send(result);
    });
    lookup_started_receiver
        .recv_timeout(TEST_SYNC_TIMEOUT)
        .map_err(|_| AppError::state("Resolver worker did not begin its blocking lookup."))?;

    for index in 0..RESOLUTION_QUEUE_CAPACITY {
        let host = format!("queued-{index}.example");
        let cancellation_checks = AtomicUsize::new(0);
        let cancel_after_enqueue = || cancellation_checks.fetch_add(1, Ordering::SeqCst) > 0;
        assert_eq!(
            resolver
                .resolve_until(
                    &host,
                    443,
                    Instant::now() + TEST_SYNC_TIMEOUT,
                    &cancel_after_enqueue,
                )
                .err(),
            Some(HostResolutionError::Cancelled)
        );
    }
    assert_eq!(
        resolver
            .resolve_until(
                "queue-overflow.example",
                443,
                Instant::now() + TEST_SYNC_TIMEOUT,
                &|| false,
            )
            .err(),
        Some(HostResolutionError::QueueFull)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the resolver worker."))?;
    let blocker_result = blocker_result_receiver
        .recv_timeout(TEST_SYNC_TIMEOUT)
        .map_err(|_| AppError::state("Blocking resolver caller did not finish."))?;
    blocker
        .join()
        .map_err(|_| AppError::state("Blocking resolver caller thread panicked."))?;
    blocker_result
        .map_err(|error| AppError::state(format!("Blocking resolver caller failed: {error:?}")))?;
    Ok(())
}
