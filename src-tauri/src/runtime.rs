//! Runtime lifecycle for outgoing captions.
//!
//! The runtime owns one microphone, one active Recognition Module, and one
//! publication policy per generation. Runtime forwards continuous microphone
//! audio and consumes normalized recognition signals; provider adapters own
//! speech units, connection attempts, and protocol I/O.
//!
//! Every utterance announced with `utterance-started` resolves with either a
//! completed caption in the caption-session aggregate or an `utterance-ended`
//! event, so the UI never waits on recognition that cannot arrive. Listening
//! indicators remain distinct lifecycle events rather than placeholder text.
//!
//! Stop is a hard cutoff: the microphone is released within one receive timeout,
//! buffered and queued speech is discarded instead of drained, and no App or
//! Chatbox caption text is committed after the stop request. A state-clearing
//! typing-off packet is sent before waiting for an STT request that is already
//! in flight, so runtime commands must run off the main thread
//! (`#[tauri::command(async)]`) to keep the window responsive during that wait.

mod coordinator;
mod output;
mod supervisor;
#[cfg(test)]
mod test_support;

pub(crate) use output::{ChatboxPublisherBoundary, RuntimeGeneration};

use output::{
    RuntimePublisherInit, initialize_runtime_publisher, publisher_boundary,
    publisher_failure_message,
};
use supervisor::run_runtime_thread;

use crate::capability_planner::{ResolvedPublicationPolicy, RuntimePlanSnapshot, plan_runtime};
use crate::caption_session::CaptionSessionStore;
use crate::chatbox::{ChatboxPacer, PublisherCloseReason, RuntimeChatboxPublisher};
#[cfg(test)]
use crate::chatbox::{completed_publisher_diagnostic, live_publisher_diagnostic};
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, emit_diagnostic,
    record_and_emit_runtime_status,
};
use crate::host_resolver::HostResolver;
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeCredentialSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase,
    RuntimeSessionSnapshot,
};
use secrecy::SecretString;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use tauri::{AppHandle, Runtime};

pub(crate) struct RuntimeManager {
    handle: Mutex<Option<RuntimeHandle>>,
    stop_epoch: AtomicU64,
    audio_probe_active: AtomicBool,
}

pub(crate) struct AudioProbeLease<'a> {
    active: &'a AtomicBool,
}

impl Drop for AudioProbeLease<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct RuntimeStartRequest {
    pub(crate) config: AppConfig,
    pub(crate) chatbox_pacer: ChatboxPacer,
    pub(crate) caption_session: CaptionSessionStore,
    pub(crate) host_resolver: HostResolver,
    pub(crate) generation_id: u64,
    pub(crate) config_revision: u64,
    pub(crate) openai_api_key: SecretString,
    pub(crate) credential: RuntimeCredentialSnapshot,
    pub(crate) expected_stop_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStartOutcome {
    Started,
    SupersededByStop,
}

struct RuntimeHandle {
    generation: RuntimeGeneration,
    publisher: Option<RuntimeChatboxPublisher>,
    join_handle: JoinHandle<()>,
}

fn resolve_runtime_publication_policy(
    runtime_plan: &RuntimePlanSnapshot,
) -> AppResult<ResolvedPublicationPolicy> {
    runtime_plan.publication.resolved_policy().ok_or_else(|| {
        AppError::config(format!(
            "The selected recognition path and publication mode are incompatible ({}).",
            runtime_plan
                .publication
                .incompatibility_code()
                .unwrap_or("publication.incompatible")
        ))
    })
}

impl Default for RuntimeManager {
    fn default() -> Self {
        Self {
            handle: Mutex::new(None),
            stop_epoch: AtomicU64::new(0),
            audio_probe_active: AtomicBool::new(false),
        }
    }
}

impl RuntimeManager {
    pub(crate) fn stop_epoch(&self) -> u64 {
        self.stop_epoch.load(Ordering::SeqCst)
    }

    pub(crate) fn stop_epoch_unchanged(&self, expected_stop_epoch: u64) -> bool {
        self.stop_epoch() == expected_stop_epoch
    }

    pub(crate) fn prepare_for_start<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(app, &mut guard)?;

        if self.audio_probe_active.load(Ordering::SeqCst) {
            return Err(AppError::runtime(
                "A microphone test is already using the selected audio input.",
            ));
        }
        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }

        Ok(())
    }

    pub(crate) fn begin_audio_probe<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> AppResult<AudioProbeLease<'_>> {
        self.audio_probe_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| AppError::runtime("A microphone test is already running."))?;
        let lease = AudioProbeLease {
            active: &self.audio_probe_active,
        };
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(app, &mut guard)?;
        if guard.is_some() {
            return Err(AppError::runtime(
                "Stop the caption runtime before testing the microphone.",
            ));
        }
        Ok(lease)
    }

    pub(crate) fn start<R: Runtime, F>(
        &self,
        app: AppHandle<R>,
        request: RuntimeStartRequest,
        install_session: F,
    ) -> AppResult<RuntimeStartOutcome>
    where
        F: FnOnce(RuntimeSessionSnapshot) -> AppResult<()>,
    {
        let RuntimeStartRequest {
            config,
            chatbox_pacer,
            caption_session,
            host_resolver,
            generation_id,
            config_revision,
            openai_api_key,
            credential,
            expected_stop_epoch,
        } = request;
        config.validate()?;
        let runtime_plan = plan_runtime(&config);
        let publication_policy = resolve_runtime_publication_policy(&runtime_plan)?;

        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        clear_finished_runtime(&app, &mut guard)?;

        // Stop increments its epoch before waiting for this handle lock. The
        // comparison and handle installation are therefore one linearized
        // decision: an earlier Start cannot come back to life after Stop has
        // already returned while Start was resolving slow desired-state I/O.
        if !self.stop_epoch_unchanged(expected_stop_epoch) {
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }

        if guard.is_some() {
            return Err(AppError::runtime("Runtime is already running."));
        }
        if self.audio_probe_active.load(Ordering::SeqCst) {
            return Err(AppError::runtime(
                "A microphone test is already using the selected audio input.",
            ));
        }

        let generation = RuntimeGeneration::activate(&app, generation_id, caption_session)?;
        let start_cancelled = || !self.stop_epoch_unchanged(expected_stop_epoch);
        let publisher_init = initialize_runtime_publisher(
            &app,
            &config.osc,
            publication_policy,
            chatbox_pacer,
            generation.clone(),
            &host_resolver,
            &start_cancelled,
        );
        if start_cancelled() {
            match &publisher_init {
                RuntimePublisherInit::Ready(publisher) => {
                    let _ = generation.request_stop(Some(publisher));
                    let _ = publisher.join();
                }
                RuntimePublisherInit::Disabled | RuntimePublisherInit::Unavailable(_) => {
                    let _ = generation.request_stop(None);
                }
            }
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }
        let requested_host = config.osc.host.clone();
        let requested_port = config.osc.port;
        let (publisher, chatbox) = match publisher_init {
            RuntimePublisherInit::Disabled => (
                None,
                RuntimeChatboxSnapshot::Disabled {
                    host: requested_host,
                    port: requested_port,
                },
            ),
            RuntimePublisherInit::Ready(publisher) => (
                Some(publisher),
                RuntimeChatboxSnapshot::Ready {
                    host: requested_host,
                    port: requested_port,
                },
            ),
            RuntimePublisherInit::Unavailable(error) => {
                emit_diagnostic(
                    &app,
                    DiagnosticUpdate::from_error(&error, "Chatbox OSC output could not start"),
                );
                (
                    None,
                    RuntimeChatboxSnapshot::Unavailable {
                        host: requested_host,
                        port: requested_port,
                        reason_code: error.code().to_string(),
                    },
                )
            }
        };

        let session = RuntimeSessionSnapshot {
            generation: generation_id,
            phase: RuntimeSessionPhase::Starting,
            started_from_config_revision: config_revision,
            selected: RuntimeSelectedConfig::from(&config),
            runtime_plan,
            credential: Some(credential),
            chatbox,
            uploads_microphone_audio: true,
        };
        if let Err(error) = install_session(session) {
            let _ = generation.request_stop(publisher_boundary(publisher.as_ref()));
            if let Some(publisher) = &publisher {
                let _ = publisher.join();
            }
            return Err(error);
        }

        let thread_generation = generation.clone();
        let thread_publisher = publisher.clone();
        let join_handle = thread::Builder::new()
            .name("vrc-live-caption-runtime".to_string())
            .spawn(move || {
                run_runtime_thread(
                    app,
                    config,
                    openai_api_key,
                    thread_publisher,
                    thread_generation,
                    host_resolver,
                )
            })
            .map_err(|error| AppError::runtime(format!("Failed to start runtime thread: {error}")));
        let join_handle = match join_handle {
            Ok(join_handle) => join_handle,
            Err(error) => {
                let _ = generation.request_stop(publisher_boundary(publisher.as_ref()));
                if let Some(publisher) = &publisher {
                    let _ = publisher.join();
                }
                return Err(error);
            }
        };

        *guard = Some(RuntimeHandle {
            generation,
            publisher,
            join_handle,
        });

        Ok(RuntimeStartOutcome::Started)
    }

    pub(crate) fn stop<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        // Publish the stop intent before waiting for the handle. A Start that
        // has not committed its handle yet observes the changed epoch and
        // aborts; a Start already inside the handle lock is stopped below.
        self.stop_epoch.fetch_add(1, Ordering::SeqCst);

        // Hold the lock through the join so a concurrent start cannot spawn a
        // new runtime while the old worker is still finishing its last request.
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;

        let Some(handle) = guard.take() else {
            record_and_emit_runtime_status(
                app,
                RuntimeStatus::Stopped,
                Some("Runtime is already stopped".to_string()),
            );
            return Ok(());
        };

        if let Err(error) = handle
            .generation
            .request_stop(publisher_boundary(handle.publisher.as_ref()))
        {
            handle.generation.cancel_work();
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        handle.publisher.as_ref(),
                        "Completed publisher could not close",
                        "Live publisher could not close",
                    ),
                ),
            );
        }
        record_and_emit_runtime_status(
            app,
            RuntimeStatus::Stopping,
            Some("Stopping runtime and discarding pending speech".to_string()),
        );

        let publisher_result = match &handle.publisher {
            Some(publisher) => publisher.join(),
            None => Ok(()),
        };
        let runtime_panicked = handle.join_handle.join().is_err();

        if let Err(error) = publisher_result {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        handle.publisher.as_ref(),
                        "Completed publisher failed while stopping",
                        "Live publisher failed while stopping",
                    ),
                ),
            );
        }

        if runtime_panicked {
            let error = AppError::runtime("Runtime thread panicked while stopping.");
            record_and_emit_runtime_status(app, RuntimeStatus::Error, Some(error.to_string()));
            return Err(error);
        }

        record_and_emit_runtime_status(
            app,
            RuntimeStatus::Stopped,
            Some("Runtime stopped".to_string()),
        );
        emit_diagnostic(
            app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Runtime,
                "runtime.stopped",
                "Runtime stopped",
                "Microphone capture has been released.",
            ),
        );

        Ok(())
    }
}

fn clear_finished_runtime<R: Runtime>(
    app: &AppHandle<R>,
    handle: &mut Option<RuntimeHandle>,
) -> AppResult<()> {
    let is_finished = handle
        .as_ref()
        .map(|handle| handle.join_handle.is_finished())
        .unwrap_or(false);

    if !is_finished {
        return Ok(());
    }

    let Some(handle) = handle.take() else {
        return Ok(());
    };

    if let Some(publisher) = &handle.publisher {
        if let Err(error) = handle
            .generation
            .close_publisher_at_boundary(Some(publisher), PublisherCloseReason::RuntimeError)
        {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        Some(publisher),
                        "Completed publisher could not close",
                        "Live publisher could not close",
                    ),
                ),
            );
        }
        if let Err(error) = publisher.join() {
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(
                    &error,
                    publisher_failure_message(
                        Some(publisher),
                        "Completed publisher failed while closing",
                        "Live publisher failed while closing",
                    ),
                ),
            );
        }
    }

    handle
        .join_handle
        .join()
        .map_err(|_| AppError::runtime("Runtime thread panicked after stopping."))
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
