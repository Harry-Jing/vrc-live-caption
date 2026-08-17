//! Bounded, deadline-aware access to the operating system hostname resolver.
//!
//! The platform resolver itself is a blocking API. Each resolver therefore owns at most one
//! lazily-started worker and a bounded queue. Callers can abandon requests without waiting for a
//! blocked lookup, while a stuck platform lookup cannot cause replacement workers to accumulate.

use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const RESOLUTION_QUEUE_CAPACITY: usize = 8;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

type Lookup = Arc<dyn Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) struct HostResolver {
    inner: Arc<ResolverInner>,
}

struct ResolverInner {
    lookup: Lookup,
    runtime: Arc<dyn ResolverRuntime>,
    // The worker is intentionally detached: joining it could reintroduce the resolver hang that
    // this boundary contains. The OnceLock prevents replacement workers from accumulating.
    worker: OnceLock<Result<SyncSender<ResolveRequest>, String>>,
}

struct ResolveRequest {
    host: String,
    port: u16,
    deadline: Instant,
    abandoned: Arc<AtomicBool>,
    response: SyncSender<ResolveOutcome>,
}

type ResolveOutcome = Result<Vec<SocketAddr>, HostResolutionError>;

trait ResolverRuntime: Send + Sync {
    // Caller and worker must share one monotonic clock. Production waits honor `timeout`; test
    // adapters may control the observation point, but must retain an independent watchdog.
    fn now(&self) -> Instant;

    fn receive(
        &self,
        receiver: &mpsc::Receiver<ResolveOutcome>,
        timeout: Duration,
    ) -> Result<ResolveOutcome, mpsc::RecvTimeoutError>;
}

struct SystemResolverRuntime;

impl ResolverRuntime for SystemResolverRuntime {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn receive(
        &self,
        receiver: &mpsc::Receiver<ResolveOutcome>,
        timeout: Duration,
    ) -> Result<ResolveOutcome, mpsc::RecvTimeoutError> {
        receiver.recv_timeout(timeout)
    }
}

struct AbandonOnDrop(Arc<AtomicBool>);

impl Drop for AbandonOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HostResolutionError {
    Cancelled,
    DeadlineExceeded,
    LookupFailed(String),
    WorkerUnavailable(String),
    QueueFull,
}

impl Default for HostResolver {
    fn default() -> Self {
        Self::with_lookup_impl(
            |host, port| {
                (host, port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect())
            },
            Arc::new(SystemResolverRuntime),
        )
    }
}

impl HostResolver {
    pub(crate) fn resolve_until(
        &self,
        host: &str,
        port: u16,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<SocketAddr>, HostResolutionError> {
        if is_cancelled() {
            return Err(HostResolutionError::Cancelled);
        }
        if self.inner.runtime.now() >= deadline {
            return Err(HostResolutionError::DeadlineExceeded);
        }
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            if let Some(error) = self.expired_error(deadline, is_cancelled) {
                return Err(error);
            }
            return Ok(vec![SocketAddr::new(ip, port)]);
        }

        let worker = self.inner.worker.get_or_init(|| {
            spawn_worker(
                Arc::clone(&self.inner.lookup),
                Arc::clone(&self.inner.runtime),
            )
        });
        if let Some(error) = self.expired_error(deadline, is_cancelled) {
            return Err(error);
        }
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => return Err(HostResolutionError::WorkerUnavailable(error.clone())),
        };
        let (response, result) = mpsc::sync_channel(1);
        let abandoned = Arc::new(AtomicBool::new(false));
        let _abandon_on_return = AbandonOnDrop(Arc::clone(&abandoned));
        let request = ResolveRequest {
            host: host.to_string(),
            port,
            deadline,
            abandoned,
            response,
        };
        let delivery = worker.try_send(request);
        if let Some(error) = self.expired_error(deadline, is_cancelled) {
            return Err(error);
        }
        match delivery {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(HostResolutionError::QueueFull),
            Err(TrySendError::Disconnected(_)) => {
                return Err(HostResolutionError::WorkerUnavailable(
                    "The hostname resolver worker stopped unexpectedly.".to_string(),
                ));
            }
        }

        loop {
            if is_cancelled() {
                return Err(HostResolutionError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(self.inner.runtime.now());
            if remaining.is_zero() {
                return Err(HostResolutionError::DeadlineExceeded);
            }
            let wait = remaining.min(CANCELLATION_POLL_INTERVAL);
            match self.inner.runtime.receive(&result, wait) {
                Ok(outcome) => {
                    if is_cancelled() {
                        return Err(HostResolutionError::Cancelled);
                    }
                    if self.inner.runtime.now() >= deadline {
                        return Err(HostResolutionError::DeadlineExceeded);
                    }
                    return outcome;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if is_cancelled() {
                        return Err(HostResolutionError::Cancelled);
                    }
                    if self.inner.runtime.now() >= deadline {
                        return Err(HostResolutionError::DeadlineExceeded);
                    }
                    return Err(HostResolutionError::WorkerUnavailable(
                        "The hostname resolver worker closed its response channel.".to_string(),
                    ));
                }
            }
        }
    }

    /// At a deadline checkpoint, cancellation is the tie-break when both states are observable.
    /// The Boolean callback cannot reconstruct which event happened first between checkpoints.
    fn expired_error(
        &self,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<HostResolutionError> {
        if self.inner.runtime.now() < deadline {
            None
        } else if is_cancelled() {
            Some(HostResolutionError::Cancelled)
        } else {
            Some(HostResolutionError::DeadlineExceeded)
        }
    }

    fn with_lookup_impl(
        lookup: impl Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
        runtime: Arc<dyn ResolverRuntime>,
    ) -> Self {
        Self {
            inner: Arc::new(ResolverInner {
                lookup: Arc::new(lookup),
                runtime,
                worker: OnceLock::new(),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_lookup(
        lookup: impl Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self::with_lookup_impl(lookup, Arc::new(SystemResolverRuntime))
    }

    #[cfg(test)]
    fn with_lookup_and_runtime(
        lookup: impl Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
        runtime: impl ResolverRuntime + 'static,
    ) -> Self {
        Self::with_lookup_impl(lookup, Arc::new(runtime))
    }

    #[cfg(test)]
    fn with_disconnected_worker() -> Self {
        let resolver = Self::with_lookup(|_, _| {
            Err(io::Error::other(
                "Disconnected test worker must not perform a lookup.",
            ))
        });
        let (sender, receiver) = mpsc::sync_channel(RESOLUTION_QUEUE_CAPACITY);
        drop(receiver);
        let _ = resolver.inner.worker.set(Ok(sender));
        resolver
    }

    #[cfg(test)]
    fn with_cached_worker_start_error() -> Self {
        let resolver = Self::with_lookup(|_, _| {
            Err(io::Error::other(
                "Unavailable test worker must not perform a lookup.",
            ))
        });
        let _ = resolver.inner.worker.set(Err(
            "Synthetic hostname resolver worker startup failure.".to_string(),
        ));
        resolver
    }
}

fn spawn_worker(
    lookup: Lookup,
    runtime: Arc<dyn ResolverRuntime>,
) -> Result<SyncSender<ResolveRequest>, String> {
    let (sender, receiver) = mpsc::sync_channel::<ResolveRequest>(RESOLUTION_QUEUE_CAPACITY);
    thread::Builder::new()
        .name("vrc-live-caption-resolver".to_string())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                if request.abandoned.load(Ordering::SeqCst) {
                    continue;
                }

                if runtime.now() >= request.deadline {
                    let _ = request
                        .response
                        .send(Err(HostResolutionError::DeadlineExceeded));
                    continue;
                }

                let result = (lookup)(&request.host, request.port)
                    .map_err(|error| HostResolutionError::LookupFailed(error.to_string()));
                if !request.abandoned.load(Ordering::SeqCst) {
                    let outcome = if runtime.now() >= request.deadline {
                        Err(HostResolutionError::DeadlineExceeded)
                    } else {
                        result
                    };
                    let _ = request.response.send(outcome);
                }
            }
        })
        .map_err(|error| format!("Failed to start the hostname resolver worker: {error}"))?;
    Ok(sender)
}

#[cfg(test)]
#[path = "host_resolver_tests.rs"]
mod tests;
