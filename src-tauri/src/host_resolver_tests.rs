use super::*;
use crate::error::{AppError, AppResult};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

const WARMUP_HOST: &str = "resolver-warmup.example";
const TEST_SYNC_TIMEOUT: Duration = Duration::from_secs(2);
const TEST_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct ManualClock {
    now: Arc<Mutex<Instant>>,
}

struct ReleaseLookupOnWaitRuntime {
    clock: ManualClock,
    release_lookup: TestGate,
}

impl ResolverRuntime for ReleaseLookupOnWaitRuntime {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<ResolveOutcome>,
        _timeout: Duration,
    ) -> Result<ResolveOutcome, mpsc::RecvTimeoutError> {
        self.release_lookup.open();
        receive_before_watchdog(receiver)
    }
}

struct AdvanceClockAfterReceiveRuntime {
    clock: ManualClock,
    observed_at: Instant,
}

struct TransitionOnReceiveRuntime {
    clock: ManualClock,
    transition: Arc<dyn Fn() + Send + Sync>,
}

impl ResolverRuntime for TransitionOnReceiveRuntime {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn receive(
        &self,
        _receiver: &mpsc::Receiver<ResolveOutcome>,
        _timeout: Duration,
    ) -> Result<ResolveOutcome, mpsc::RecvTimeoutError> {
        (self.transition)();
        Err(mpsc::RecvTimeoutError::Timeout)
    }
}

struct BlockingReceiveRuntime {
    clock: ManualClock,
    wait_count: Arc<(Mutex<usize>, Condvar)>,
}

impl ResolverRuntime for BlockingReceiveRuntime {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<ResolveOutcome>,
        _timeout: Duration,
    ) -> Result<ResolveOutcome, mpsc::RecvTimeoutError> {
        let (lock, wake) = &*self.wait_count;
        match lock.lock() {
            Ok(mut count) => {
                *count += 1;
                wake.notify_all();
            }
            Err(poisoned) => {
                *poisoned.into_inner() += 1;
                wake.notify_all();
            }
        }
        receive_before_watchdog(receiver)
    }
}

#[derive(Clone)]
struct TestGate {
    state: Arc<(Mutex<bool>, Condvar)>,
}

impl TestGate {
    fn closed() -> Self {
        Self {
            state: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    fn open(&self) {
        let (lock, wake) = &*self.state;
        match lock.lock() {
            Ok(mut is_open) => {
                *is_open = true;
                wake.notify_all();
            }
            Err(poisoned) => {
                *poisoned.into_inner() = true;
                wake.notify_all();
            }
        }
    }

    fn wait(&self) -> io::Result<()> {
        let (lock, wake) = &*self.state;
        let is_open = lock
            .lock()
            .map_err(|_| io::Error::other("Test gate lock was poisoned."))?;
        let (is_open, _) = wake
            .wait_timeout_while(is_open, TEST_WATCHDOG_TIMEOUT, |is_open| !*is_open)
            .map_err(|_| io::Error::other("Test gate wait was poisoned."))?;
        if *is_open {
            Ok(())
        } else {
            Err(io::Error::other("Test gate was not opened."))
        }
    }
}

struct OpenGateOnDrop(TestGate);

impl Drop for OpenGateOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

impl ResolverRuntime for AdvanceClockAfterReceiveRuntime {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<ResolveOutcome>,
        _timeout: Duration,
    ) -> Result<ResolveOutcome, mpsc::RecvTimeoutError> {
        let outcome = receive_before_watchdog(receiver)?;
        self.clock.advance_to(self.observed_at);
        Ok(outcome)
    }
}

impl ManualClock {
    fn new(now: Instant) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    fn now(&self) -> Instant {
        match self.now.lock() {
            Ok(now) => *now,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    fn advance_to(&self, now: Instant) {
        match self.now.lock() {
            Ok(mut current) => *current = (*current).max(now),
            Err(poisoned) => {
                let mut current = poisoned.into_inner();
                *current = (*current).max(now);
            }
        }
    }
}

fn wait_for_count(milestone: &Arc<(Mutex<usize>, Condvar)>, expected: usize) -> AppResult<()> {
    let (lock, wake) = &**milestone;
    let count = lock
        .lock()
        .map_err(|_| AppError::state("Test milestone lock was poisoned."))?;
    let (count, _) = wake
        .wait_timeout_while(count, TEST_WATCHDOG_TIMEOUT, |count| *count < expected)
        .map_err(|_| AppError::state("Test milestone wait was poisoned."))?;
    if *count < expected {
        return Err(AppError::state(
            "Resolver callers did not enter their response waits.",
        ));
    }
    Ok(())
}

fn receive_before_watchdog(
    receiver: &mpsc::Receiver<ResolveOutcome>,
) -> Result<ResolveOutcome, mpsc::RecvTimeoutError> {
    receiver
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
}

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
fn literal_ipv4_and_ipv6_addresses_bypass_the_worker_lookup() -> AppResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup_calls = Arc::clone(&calls);
    let resolver = HostResolver::with_lookup(move |_, port| {
        lookup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(loopback_result(port))
    });

    let ipv4_addresses = resolver
        .resolve_until(
            "127.0.0.1",
            9000,
            Instant::now() + Duration::from_secs(1),
            &|| false,
        )
        .map_err(|error| AppError::state(format!("IPv4 resolution failed: {error:?}")))?;
    let ipv6_addresses = resolver
        .resolve_until(
            "[::1]",
            9001,
            Instant::now() + Duration::from_secs(1),
            &|| false,
        )
        .map_err(|error| AppError::state(format!("IPv6 resolution failed: {error:?}")))?;

    assert_eq!(ipv4_addresses, loopback_result(9000));
    assert_eq!(
        ipv6_addresses,
        vec![SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 9001))]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn cancellation_is_the_tie_break_when_both_states_are_visible_at_entry() {
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(1);
    let clock = ManualClock::new(started_at);
    let resolver = HostResolver::with_lookup_and_runtime(
        |_, port| Ok(loopback_result(port)),
        BlockingReceiveRuntime {
            clock: clock.clone(),
            wait_count: Arc::new((Mutex::new(0), Condvar::new())),
        },
    );

    clock.advance_to(deadline);
    assert_eq!(
        resolver
            .resolve_until("simultaneous.example", 443, deadline, &|| true)
            .err(),
        Some(HostResolutionError::Cancelled)
    );
}

#[test]
fn in_flight_cancellation_and_deadline_order_preserves_the_first_observed_terminal_state() {
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_transition = Arc::clone(&cancelled);
    let cancel_clock = ManualClock::new(started_at);
    let cancel_resolver = HostResolver::with_lookup_and_runtime(
        |_, port| Ok(loopback_result(port)),
        TransitionOnReceiveRuntime {
            clock: cancel_clock.clone(),
            transition: Arc::new(move || cancel_transition.store(true, Ordering::SeqCst)),
        },
    );

    let cancelled_result =
        cancel_resolver.resolve_until("cancelled-in-flight.example", 443, deadline, &|| {
            cancelled.load(Ordering::SeqCst)
        });
    cancel_clock.advance_to(deadline);
    assert_eq!(cancelled_result.err(), Some(HostResolutionError::Cancelled));

    let deadline_clock = ManualClock::new(started_at);
    let advance_clock = deadline_clock.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let deadline_resolver = HostResolver::with_lookup_and_runtime(
        |_, port| Ok(loopback_result(port)),
        TransitionOnReceiveRuntime {
            clock: deadline_clock,
            transition: Arc::new(move || advance_clock.advance_to(deadline)),
        },
    );

    let deadline_result =
        deadline_resolver.resolve_until("deadline-in-flight.example", 443, deadline, &|| {
            cancelled.load(Ordering::SeqCst)
        });
    cancelled.store(true, Ordering::SeqCst);
    assert_eq!(
        deadline_result.err(),
        Some(HostResolutionError::DeadlineExceeded)
    );
}

#[test]
fn worker_disconnection_before_the_deadline_remains_unavailable() {
    let resolver = HostResolver::with_disconnected_worker();

    for host in ["first.example", "no-replacement.example"] {
        assert!(matches!(
            resolver
                .resolve_until(host, 443, Instant::now() + TEST_SYNC_TIMEOUT, &|| false,)
                .err(),
            Some(HostResolutionError::WorkerUnavailable(_))
        ));
    }
}

#[test]
fn cached_worker_start_failure_before_the_deadline_remains_unavailable() {
    let resolver = HostResolver::with_cached_worker_start_error();

    for host in ["first.example", "cached-failure.example"] {
        assert!(matches!(
            resolver
                .resolve_until(host, 443, Instant::now() + TEST_SYNC_TIMEOUT, &|| false,)
                .err(),
            Some(HostResolutionError::WorkerUnavailable(_))
        ));
    }
}

#[test]
fn lookup_completing_after_the_deadline_is_rejected() {
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(1);
    let clock = ManualClock::new(started_at);
    let release_lookup = TestGate::closed();
    let lookup_release = release_lookup.clone();
    let lookup_clock = clock.clone();
    let resolver = HostResolver::with_lookup_and_runtime(
        move |_, port| {
            lookup_release.wait()?;
            lookup_clock.advance_to(deadline);
            Ok(loopback_result(port))
        },
        ReleaseLookupOnWaitRuntime {
            clock,
            release_lookup,
        },
    );

    assert_eq!(
        resolver
            .resolve_until("late.example", 443, deadline, &|| false)
            .err(),
        Some(HostResolutionError::DeadlineExceeded)
    );
}

#[test]
fn successful_result_observed_after_the_deadline_is_rejected() {
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(1);
    let clock = ManualClock::new(started_at);
    let resolver = HostResolver::with_lookup_and_runtime(
        |_, port| Ok(loopback_result(port)),
        AdvanceClockAfterReceiveRuntime {
            clock,
            observed_at: deadline,
        },
    );

    assert_eq!(
        resolver
            .resolve_until("ready-before-delay.example", 443, deadline, &|| false)
            .err(),
        Some(HostResolutionError::DeadlineExceeded)
    );
}

#[test]
fn queued_request_expired_before_worker_start_reports_the_deadline() -> AppResult<()> {
    let started_at = Instant::now();
    let queued_deadline = started_at + Duration::from_secs(1);
    let blocker_deadline = started_at + Duration::from_secs(2);
    let clock = ManualClock::new(started_at);
    let wait_count = Arc::new((Mutex::new(0), Condvar::new()));
    let lookup_release = TestGate::closed();
    let _release_on_return = OpenGateOnDrop(lookup_release.clone());
    let blocker_gate = lookup_release.clone();
    let queued_lookups = Arc::new(AtomicUsize::new(0));
    let lookup_count = Arc::clone(&queued_lookups);
    let (blocker_started, blocker_entered) = mpsc::sync_channel(1);
    let resolver = HostResolver::with_lookup_and_runtime(
        move |host, port| {
            if host == "worker-blocker.example" {
                let _ = blocker_started.send(());
                blocker_gate.wait()?;
            } else if host == "queued-expired.example" {
                lookup_count.fetch_add(1, Ordering::SeqCst);
            }
            Ok(loopback_result(port))
        },
        BlockingReceiveRuntime {
            clock: clock.clone(),
            wait_count: Arc::clone(&wait_count),
        },
    );

    let blocker_resolver = resolver.clone();
    let (blocker_result_sender, blocker_result_receiver) = mpsc::sync_channel(1);
    let blocker = thread::spawn(move || {
        let _ = blocker_result_sender.send(blocker_resolver.resolve_until(
            "worker-blocker.example",
            443,
            blocker_deadline,
            &|| false,
        ));
    });
    blocker_entered
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("Resolver worker did not enter the blocking lookup."))?;

    let queued_resolver = resolver.clone();
    let (queued_result_sender, queued_result_receiver) = mpsc::sync_channel(1);
    let queued = thread::spawn(move || {
        let _ = queued_result_sender.send(queued_resolver.resolve_until(
            "queued-expired.example",
            443,
            queued_deadline,
            &|| false,
        ));
    });
    wait_for_count(&wait_count, 2)?;
    clock.advance_to(queued_deadline);
    lookup_release.open();

    let blocker_result = blocker_result_receiver
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("Blocking resolver caller did not finish."))?;
    let queued_result = queued_result_receiver
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("Queued resolver caller did not finish."))?;
    blocker
        .join()
        .map_err(|_| AppError::state("Blocking resolver caller panicked."))?;
    queued
        .join()
        .map_err(|_| AppError::state("Queued resolver caller panicked."))?;

    assert_eq!(blocker_result, Ok(loopback_result(443)));
    assert_eq!(
        queued_result.err(),
        Some(HostResolutionError::DeadlineExceeded)
    );
    assert_eq!(queued_lookups.load(Ordering::SeqCst), 0);
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
// Temporary #41 acceptance probe; removed before this PR is ready.
#[test]
fn ci_required_failure_probe() {
    assert!(std::env::var_os("VRC_CI_GATE_PROBE").is_none());
}
