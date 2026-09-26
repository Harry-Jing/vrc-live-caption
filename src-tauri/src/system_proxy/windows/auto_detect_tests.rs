use super::*;
use crate::error::{AppError, AppResult};
use std::sync::Condvar;
use std::sync::atomic::AtomicUsize;

const TEST_DEADLINE: Duration = Duration::from_secs(1);
const TEST_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct ManualClock {
    now: Arc<Mutex<Instant>>,
}

impl ManualClock {
    fn new(now: Instant) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn advance_to(&self, now: Instant) {
        let mut current = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *current = (*current).max(now);
    }
}

/// Observes answers against a manual clock. The watchdog bounds a lost answer
/// and surfaces it as a failed detection.
struct ManualRuntime {
    clock: ManualClock,
}

impl DetectorRuntime for ManualRuntime {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<AutoProxyDetection>,
        _timeout: Duration,
    ) -> Result<AutoProxyDetection, mpsc::RecvTimeoutError> {
        receive_before_watchdog(receiver)
    }
}

/// Moves policy time to `expire_at` during the caller's first wait, but only
/// after the worker has entered the platform probe.
struct ExpireOnFirstWaitRuntime {
    clock: ManualClock,
    probe_entered: Mutex<mpsc::Receiver<()>>,
    expire_at: Instant,
    waits: AtomicUsize,
}

impl DetectorRuntime for ExpireOnFirstWaitRuntime {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<AutoProxyDetection>,
        _timeout: Duration,
    ) -> Result<AutoProxyDetection, mpsc::RecvTimeoutError> {
        if self.waits.fetch_add(1, Ordering::SeqCst) > 0 {
            return receive_before_watchdog(receiver);
        }
        let entered = self
            .probe_entered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(TEST_WATCHDOG_TIMEOUT);
        if entered.is_err() {
            return Err(mpsc::RecvTimeoutError::Disconnected);
        }
        self.clock.advance_to(self.expire_at);
        Err(mpsc::RecvTimeoutError::Timeout)
    }
}

fn receive_before_watchdog(
    receiver: &mpsc::Receiver<AutoProxyDetection>,
) -> Result<AutoProxyDetection, mpsc::RecvTimeoutError> {
    receiver
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
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
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        wake.notify_all();
    }

    /// Returns whether the gate opened before the watchdog.
    fn wait(&self) -> bool {
        let (lock, wake) = &*self.state;
        let is_open = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (is_open, _) = wake
            .wait_timeout_while(is_open, TEST_WATCHDOG_TIMEOUT, |is_open| !*is_open)
            .unwrap_or_else(PoisonError::into_inner);
        *is_open
    }
}

struct OpenGateOnDrop(TestGate);

impl Drop for OpenGateOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

/// A platform probe that answers `outcome` and counts its calls.
fn counting_probe(
    outcome: ProbeOutcome,
) -> (
    Arc<AtomicUsize>,
    impl Fn() -> ProbeOutcome + Send + Sync + 'static,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = Arc::clone(&calls);
    let probe = move || {
        probe_calls.fetch_add(1, Ordering::SeqCst);
        outcome
    };
    (calls, probe)
}

/// A platform probe that reports each entry, then holds the worker until
/// `release` opens, as a slow DHCP or DNS discovery would.
struct BlockingProbe {
    entered: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
    entry: mpsc::Receiver<()>,
}

fn blocking_probe(
    release: TestGate,
    outcome: ProbeOutcome,
) -> (
    BlockingProbe,
    impl Fn() -> ProbeOutcome + Send + Sync + 'static,
) {
    let entered = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let (entry_sender, entry) = mpsc::sync_channel(DETECTION_QUEUE_CAPACITY + 1);
    let probe_entered = Arc::clone(&entered);
    let probe_completed = Arc::clone(&completed);
    let probe = move || {
        probe_entered.fetch_add(1, Ordering::SeqCst);
        let _ = entry_sender.try_send(());
        let released = release.wait();
        probe_completed.fetch_add(1, Ordering::SeqCst);
        if released {
            outcome
        } else {
            ProbeOutcome::Failed
        }
    };
    (
        BlockingProbe {
            entered,
            completed,
            entry,
        },
        probe,
    )
}

#[test]
fn a_no_script_answer_is_reused_until_its_period_ends() {
    let started_at = Instant::now();
    let clock = ManualClock::new(started_at);
    let (probes, probe) = counting_probe(ProbeOutcome::NoScript);
    let detector = AutoProxyDetector::with_runtime(
        probe,
        Arc::new(ManualRuntime {
            clock: clock.clone(),
        }),
    );

    assert_eq!(
        detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
        AutoProxyDetection::NoScript
    );
    assert_eq!(probes.load(Ordering::SeqCst), 1);

    clock.advance_to(started_at + NO_SCRIPT_REUSE_PERIOD - Duration::from_millis(1));
    assert_eq!(
        detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
        AutoProxyDetection::NoScript
    );
    assert_eq!(probes.load(Ordering::SeqCst), 1);

    clock.advance_to(started_at + NO_SCRIPT_REUSE_PERIOD);
    assert_eq!(
        detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
        AutoProxyDetection::NoScript
    );
    assert_eq!(probes.load(Ordering::SeqCst), 2);
}

#[test]
fn found_scripts_and_failures_are_probed_again() {
    let clock = ManualClock::new(Instant::now());
    let answers = [
        ProbeOutcome::ScriptFound,
        ProbeOutcome::Failed,
        ProbeOutcome::ScriptFound,
    ];
    let probes = Arc::new(AtomicUsize::new(0));
    let probe_calls = Arc::clone(&probes);
    let detector = AutoProxyDetector::with_runtime(
        move || {
            let call = probe_calls.fetch_add(1, Ordering::SeqCst);
            answers.get(call).copied().unwrap_or(ProbeOutcome::Failed)
        },
        Arc::new(ManualRuntime {
            clock: clock.clone(),
        }),
    );

    for expected in [
        AutoProxyDetection::ScriptFound,
        AutoProxyDetection::Failed,
        AutoProxyDetection::ScriptFound,
    ] {
        assert_eq!(
            detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
            expected
        );
    }
    assert_eq!(probes.load(Ordering::SeqCst), 3);
}

#[test]
fn the_deadline_returns_while_the_probe_is_blocked_and_its_late_answer_is_reused() {
    let started_at = Instant::now();
    let deadline = started_at + TEST_DEADLINE;
    let clock = ManualClock::new(started_at);
    let release = TestGate::closed();
    let _release_on_return = OpenGateOnDrop(release.clone());
    let (probe_state, probe) = blocking_probe(release.clone(), ProbeOutcome::NoScript);
    let detector = AutoProxyDetector::with_runtime(
        probe,
        Arc::new(ExpireOnFirstWaitRuntime {
            clock: clock.clone(),
            probe_entered: Mutex::new(probe_state.entry),
            expire_at: deadline,
            waits: AtomicUsize::new(0),
        }),
    );

    assert_eq!(
        detector.detect_until(deadline, &|| false),
        AutoProxyDetection::Unfinished
    );
    assert_eq!(probe_state.entered.load(Ordering::SeqCst), 1);
    assert_eq!(probe_state.completed.load(Ordering::SeqCst), 0);

    // The abandoned probe still finishes on the worker, and its answer serves
    // the retry without another discovery.
    release.open();
    assert_eq!(
        detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
        AutoProxyDetection::NoScript
    );
    assert_eq!(probe_state.entered.load(Ordering::SeqCst), 1);
    assert_eq!(probe_state.completed.load(Ordering::SeqCst), 1);
}

#[test]
fn the_detection_budget_caps_a_later_caller_deadline() {
    let started_at = Instant::now();
    let clock = ManualClock::new(started_at);
    let release = TestGate::closed();
    let _release_on_return = OpenGateOnDrop(release.clone());
    let (probe_state, probe) = blocking_probe(release, ProbeOutcome::NoScript);
    let detector = AutoProxyDetector::with_runtime(
        probe,
        Arc::new(ExpireOnFirstWaitRuntime {
            clock,
            probe_entered: Mutex::new(probe_state.entry),
            expire_at: started_at + DETECTION_BUDGET,
            waits: AtomicUsize::new(0),
        }),
    );

    assert_eq!(
        detector.detect_until(started_at + Duration::from_secs(60), &|| false),
        AutoProxyDetection::Unfinished
    );
    assert_eq!(probe_state.completed.load(Ordering::SeqCst), 0);
}

#[test]
fn an_answer_observed_after_the_deadline_is_discarded_but_reused() {
    let started_at = Instant::now();
    let deadline = started_at + TEST_DEADLINE;
    let clock = ManualClock::new(started_at);
    let probe_clock = clock.clone();
    let probes = Arc::new(AtomicUsize::new(0));
    let probe_calls = Arc::clone(&probes);
    let detector = AutoProxyDetector::with_runtime(
        move || {
            probe_calls.fetch_add(1, Ordering::SeqCst);
            probe_clock.advance_to(deadline);
            ProbeOutcome::NoScript
        },
        Arc::new(ManualRuntime {
            clock: clock.clone(),
        }),
    );

    assert_eq!(
        detector.detect_until(deadline, &|| false),
        AutoProxyDetection::Unfinished
    );
    assert_eq!(
        detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
        AutoProxyDetection::NoScript
    );
    assert_eq!(probes.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_returns_while_the_probe_is_blocked() -> AppResult<()> {
    let release = TestGate::closed();
    let _release_on_return = OpenGateOnDrop(release.clone());
    let (probe_state, probe) = blocking_probe(release, ProbeOutcome::NoScript);
    let detector = Arc::new(AutoProxyDetector::new(probe));
    let cancelled = Arc::new(AtomicBool::new(false));
    let caller_cancelled = Arc::clone(&cancelled);
    let caller_detector = Arc::clone(&detector);
    let (result_sender, result) = mpsc::sync_channel(1);
    let caller = thread::spawn(move || {
        let detection = caller_detector.detect_until(Instant::now() + DETECTION_BUDGET, &|| {
            caller_cancelled.load(Ordering::SeqCst)
        });
        let _ = result_sender.send(detection);
    });

    probe_state
        .entry
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("The detection worker did not enter the probe."))?;
    cancelled.store(true, Ordering::SeqCst);
    let detection = result
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("A cancelled caller kept waiting for the probe."))?;
    caller
        .join()
        .map_err(|_| AppError::state("The detection caller thread panicked."))?;

    assert_eq!(detection, AutoProxyDetection::Cancelled);
    assert_eq!(probe_state.completed.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn one_blocked_probe_has_a_bounded_request_queue() -> AppResult<()> {
    let clock = ManualClock::new(Instant::now());
    let release = TestGate::closed();
    let _release_on_return = OpenGateOnDrop(release.clone());
    let (probe_state, probe) = blocking_probe(release.clone(), ProbeOutcome::NoScript);
    let detector = Arc::new(AutoProxyDetector::with_runtime(
        probe,
        Arc::new(ManualRuntime {
            clock: clock.clone(),
        }),
    ));

    let blocker_detector = Arc::clone(&detector);
    let blocker_deadline = clock.now() + TEST_DEADLINE;
    let (blocker_sender, blocker_result) = mpsc::sync_channel(1);
    let blocker = thread::spawn(move || {
        let _ = blocker_sender.send(blocker_detector.detect_until(blocker_deadline, &|| false));
    });
    probe_state
        .entry
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("The detection worker did not enter the probe."))?;

    for _ in 0..DETECTION_QUEUE_CAPACITY {
        let cancellation_checks = AtomicUsize::new(0);
        let cancel_after_enqueue = || cancellation_checks.fetch_add(1, Ordering::SeqCst) > 0;
        assert_eq!(
            detector.detect_until(clock.now() + TEST_DEADLINE, &cancel_after_enqueue),
            AutoProxyDetection::Cancelled
        );
    }
    assert_eq!(
        detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
        AutoProxyDetection::Unfinished
    );
    assert_eq!(probe_state.entered.load(Ordering::SeqCst), 1);

    release.open();
    let blocker_detection = blocker_result
        .recv_timeout(TEST_WATCHDOG_TIMEOUT)
        .map_err(|_| AppError::state("The blocked detection caller did not finish."))?;
    blocker
        .join()
        .map_err(|_| AppError::state("The blocked detection caller thread panicked."))?;
    assert_eq!(blocker_detection, AutoProxyDetection::NoScript);
    Ok(())
}

#[test]
fn an_unavailable_worker_fails_closed() {
    let clock = ManualClock::new(Instant::now());
    let (never_started_probes, probe) = counting_probe(ProbeOutcome::NoScript);
    let never_started = AutoProxyDetector::with_runtime(
        probe,
        Arc::new(ManualRuntime {
            clock: clock.clone(),
        }),
    );
    let _ = never_started.worker.set(None);

    let (disconnected_probes, probe) = counting_probe(ProbeOutcome::NoScript);
    let disconnected = AutoProxyDetector::with_runtime(
        probe,
        Arc::new(ManualRuntime {
            clock: clock.clone(),
        }),
    );
    let (sender, receiver) = mpsc::sync_channel(DETECTION_QUEUE_CAPACITY);
    drop(receiver);
    let _ = disconnected.worker.set(Some(sender));

    for detector in [&never_started, &disconnected] {
        assert_eq!(
            detector.detect_until(clock.now() + TEST_DEADLINE, &|| false),
            AutoProxyDetection::Failed
        );
    }
    assert_eq!(never_started_probes.load(Ordering::SeqCst), 0);
    assert_eq!(disconnected_probes.load(Ordering::SeqCst), 0);
}
