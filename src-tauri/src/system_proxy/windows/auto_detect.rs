//! Bounded, reusable access to Windows automatic proxy detection (WPAD).
//!
//! WinHTTP's script discovery blocks for as long as its DHCP and DNS probes
//! take and cannot be cancelled. The detector therefore owns at most one
//! lazily-started worker behind a bounded queue: callers wait only until their
//! own deadline or cancellation, abandoned requests are skipped rather than
//! joined, and a stuck probe cannot make replacement workers accumulate.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Longest wait one route selection spends on discovery. The worker keeps
/// probing after a caller gives up, so an unfinished answer is retryable.
const DETECTION_BUDGET: Duration = Duration::from_secs(5);
/// How long a probe that found no script answers later selections, so
/// Translation attempts and Realtime reconnects do not each pay for discovery.
/// .NET's `HttpWindowsProxy` likewise waits two minutes before detecting again.
const NO_SCRIPT_REUSE_PERIOD: Duration = Duration::from_secs(120);
const DETECTION_QUEUE_CAPACITY: usize = 4;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

type Probe = Arc<dyn Fn() -> ProbeOutcome + Send + Sync + 'static>;

/// The result of one completed platform discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProbeOutcome {
    NoScript,
    ScriptFound,
    Failed,
}

/// What one route selection learned before its deadline or cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AutoProxyDetection {
    /// Discovery completed, now or within the reuse period, without a script.
    NoScript,
    ScriptFound,
    /// Discovery, or the worker that runs it, failed.
    Failed,
    /// No answer arrived before the deadline, or earlier requests still fill
    /// the queue behind a slow probe.
    Unfinished,
    Cancelled,
}

impl From<ProbeOutcome> for AutoProxyDetection {
    fn from(outcome: ProbeOutcome) -> Self {
        match outcome {
            ProbeOutcome::NoScript => Self::NoScript,
            ProbeOutcome::ScriptFound => Self::ScriptFound,
            ProbeOutcome::Failed => Self::Failed,
        }
    }
}

pub(super) struct AutoProxyDetector {
    probe: Probe,
    runtime: Arc<dyn DetectorRuntime>,
    no_script: Arc<NoScriptAnswer>,
    // The worker is intentionally detached: joining it could reintroduce the
    // discovery hang this boundary contains. The OnceLock prevents replacement
    // workers from accumulating; a worker that could not start stays absent.
    worker: OnceLock<Option<SyncSender<DetectionRequest>>>,
}

struct DetectionRequest {
    deadline: Instant,
    abandoned: Arc<AtomicBool>,
    response: SyncSender<AutoProxyDetection>,
}

trait DetectorRuntime: Send + Sync {
    // Caller, worker, and the reuse period share one monotonic clock.
    // Production waits honor `timeout`; test adapters may control the
    // observation point, but must retain an independent watchdog.
    fn now(&self) -> Instant;

    fn receive(
        &self,
        receiver: &mpsc::Receiver<AutoProxyDetection>,
        timeout: Duration,
    ) -> Result<AutoProxyDetection, mpsc::RecvTimeoutError>;
}

struct SystemDetectorRuntime;

impl DetectorRuntime for SystemDetectorRuntime {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<AutoProxyDetection>,
        timeout: Duration,
    ) -> Result<AutoProxyDetection, mpsc::RecvTimeoutError> {
        receiver.recv_timeout(timeout)
    }
}

/// The only reusable answer. A found script or failure fails the connection
/// closed, and an unfinished probe must be asked again.
#[derive(Default)]
struct NoScriptAnswer {
    observed_at: Mutex<Option<Instant>>,
}

impl NoScriptAnswer {
    fn is_current(&self, now: Instant) -> bool {
        let observed_at = *self
            .observed_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        observed_at.is_some_and(|observed_at| {
            now.saturating_duration_since(observed_at) < NO_SCRIPT_REUSE_PERIOD
        })
    }

    fn record(&self, now: Instant) {
        *self
            .observed_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(now);
    }
}

struct AbandonOnDrop(Arc<AtomicBool>);

impl Drop for AbandonOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl AutoProxyDetector {
    pub(super) fn new(probe: impl Fn() -> ProbeOutcome + Send + Sync + 'static) -> Self {
        Self::with_runtime(probe, Arc::new(SystemDetectorRuntime))
    }

    fn with_runtime(
        probe: impl Fn() -> ProbeOutcome + Send + Sync + 'static,
        runtime: Arc<dyn DetectorRuntime>,
    ) -> Self {
        Self {
            probe: Arc::new(probe),
            runtime,
            no_script: Arc::default(),
            worker: OnceLock::new(),
        }
    }

    /// Waits until the caller's deadline, capped by `DETECTION_BUDGET`, for
    /// discovery to answer, and discards an answer observed after it.
    /// Cancellation wins when both terminal states are visible at a
    /// checkpoint. Abandoning a request never joins the platform probe.
    pub(super) fn detect_until(
        &self,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AutoProxyDetection {
        if is_cancelled() {
            return AutoProxyDetection::Cancelled;
        }
        let now = self.runtime.now();
        let deadline = now
            .checked_add(DETECTION_BUDGET)
            .map_or(deadline, |budget_end| budget_end.min(deadline));
        if now >= deadline {
            return AutoProxyDetection::Unfinished;
        }
        if self.no_script.is_current(now) {
            return AutoProxyDetection::NoScript;
        }

        let worker = self.worker.get_or_init(|| {
            spawn_worker(
                Arc::clone(&self.probe),
                Arc::clone(&self.runtime),
                Arc::clone(&self.no_script),
            )
        });
        let Some(worker) = worker else {
            return AutoProxyDetection::Failed;
        };
        let (response, answer) = mpsc::sync_channel(1);
        let abandoned = Arc::new(AtomicBool::new(false));
        let _abandon_on_return = AbandonOnDrop(Arc::clone(&abandoned));
        match worker.try_send(DetectionRequest {
            deadline,
            abandoned,
            response,
        }) {
            Ok(()) => {}
            // Earlier requests are still queued behind a slow probe that the
            // platform keeps running, so a later attempt can still succeed.
            Err(TrySendError::Full(_)) => return AutoProxyDetection::Unfinished,
            Err(TrySendError::Disconnected(_)) => return AutoProxyDetection::Failed,
        }

        loop {
            if is_cancelled() {
                return AutoProxyDetection::Cancelled;
            }
            let remaining = deadline.saturating_duration_since(self.runtime.now());
            if remaining.is_zero() {
                return AutoProxyDetection::Unfinished;
            }
            let detection = match self
                .runtime
                .receive(&answer, remaining.min(CANCELLATION_POLL_INTERVAL))
            {
                Ok(detection) => detection,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // The worker stopped without answering a request it accepted.
                Err(mpsc::RecvTimeoutError::Disconnected) => AutoProxyDetection::Failed,
            };
            if is_cancelled() {
                return AutoProxyDetection::Cancelled;
            }
            if self.runtime.now() >= deadline {
                return AutoProxyDetection::Unfinished;
            }
            return detection;
        }
    }
}

fn spawn_worker(
    probe: Probe,
    runtime: Arc<dyn DetectorRuntime>,
    no_script: Arc<NoScriptAnswer>,
) -> Option<SyncSender<DetectionRequest>> {
    let (sender, receiver) = mpsc::sync_channel::<DetectionRequest>(DETECTION_QUEUE_CAPACITY);
    thread::Builder::new()
        .name("vrc-live-caption-proxy-detection".to_string())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                if request.abandoned.load(Ordering::SeqCst) {
                    continue;
                }
                let detection = if no_script.is_current(runtime.now()) {
                    // A probe finished for an earlier request answers this one.
                    AutoProxyDetection::NoScript
                } else if runtime.now() >= request.deadline {
                    AutoProxyDetection::Unfinished
                } else {
                    let outcome = probe();
                    if outcome == ProbeOutcome::NoScript {
                        // Recorded even if the requester has gone, so its
                        // retry does not pay for discovery again.
                        no_script.record(runtime.now());
                    }
                    outcome.into()
                };
                if !request.abandoned.load(Ordering::SeqCst) {
                    let _ = request.response.send(detection);
                }
            }
        })
        .ok()?;
    Some(sender)
}

#[cfg(test)]
#[path = "auto_detect_tests.rs"]
mod tests;
