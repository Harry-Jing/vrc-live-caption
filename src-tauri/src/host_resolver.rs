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
    // The worker is intentionally detached: joining it could reintroduce the resolver hang that
    // this boundary contains. The OnceLock prevents replacement workers from accumulating.
    worker: OnceLock<Result<SyncSender<ResolveRequest>, String>>,
}

struct ResolveRequest {
    host: String,
    port: u16,
    deadline: Instant,
    abandoned: Arc<AtomicBool>,
    response: SyncSender<Result<Vec<SocketAddr>, String>>,
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
        Self::with_lookup_impl(|host, port| {
            (host, port)
                .to_socket_addrs()
                .map(|addresses| addresses.collect())
        })
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
        if Instant::now() >= deadline {
            return Err(HostResolutionError::DeadlineExceeded);
        }
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }

        let worker = match self
            .inner
            .worker
            .get_or_init(|| spawn_worker(Arc::clone(&self.inner.lookup)))
        {
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
        match worker.try_send(request) {
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
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(HostResolutionError::DeadlineExceeded);
            }
            let wait = remaining.min(CANCELLATION_POLL_INTERVAL);
            match result.recv_timeout(wait) {
                Ok(Ok(addresses)) => return Ok(addresses),
                Ok(Err(error)) => return Err(HostResolutionError::LookupFailed(error)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(HostResolutionError::WorkerUnavailable(
                        "The hostname resolver worker closed its response channel.".to_string(),
                    ));
                }
            }
        }
    }

    fn with_lookup_impl(
        lookup: impl Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(ResolverInner {
                lookup: Arc::new(lookup),
                worker: OnceLock::new(),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_lookup(
        lookup: impl Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self::with_lookup_impl(lookup)
    }
}

fn spawn_worker(lookup: Lookup) -> Result<SyncSender<ResolveRequest>, String> {
    let (sender, receiver) = mpsc::sync_channel::<ResolveRequest>(RESOLUTION_QUEUE_CAPACITY);
    thread::Builder::new()
        .name("vrc-live-caption-resolver".to_string())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                if request.abandoned.load(Ordering::SeqCst) || Instant::now() >= request.deadline {
                    continue;
                }

                let result =
                    (lookup)(&request.host, request.port).map_err(|error| error.to_string());
                if !request.abandoned.load(Ordering::SeqCst) {
                    let _ = request.response.send(result);
                }
            }
        })
        .map_err(|error| format!("Failed to start the hostname resolver worker: {error}"))?;
    Ok(sender)
}

#[cfg(test)]
#[path = "host_resolver_tests.rs"]
mod tests;
