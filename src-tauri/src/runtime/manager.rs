use super::coordinator::RuntimeExecution;
use super::output::{ChatboxPublicationInit, RuntimeGeneration, initialize_chatbox_publication};
use super::supervisor::run_runtime_thread;

use crate::caption::CaptionAggregateStore;
use crate::caption_pipeline::{plan_caption_pipeline, publication_timing_for_start};
use crate::chatbox::{ChatboxPacer, ChatboxPublication};
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, emit_diagnostic, record_and_emit_runtime_status,
};
use crate::host_resolver::HostResolver;
use crate::recognition::RecognitionModule;
use crate::runtime_control::{
    ChatboxPublicationSnapshot, RuntimeGenerationCredentialSnapshot, RuntimeGenerationPhase,
    RuntimeGenerationSelection, RuntimeGenerationSnapshot, RuntimeStatus, RuntimeStatusRecorder,
};
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
    pub(crate) caption_aggregate: CaptionAggregateStore,
    pub(crate) chatbox_host_resolver: HostResolver,
    pub(crate) prepared_recognition: PreparedRecognition,
    pub(crate) generation_id: u64,
    pub(crate) config_revision: u64,
    pub(crate) status_recorder: RuntimeStatusRecorder,
    pub(crate) expected_stop_epoch: u64,
}

/// A recognition Module bound to the generation metadata implied by how it was
/// prepared at the desktop composition boundary.
///
/// Runtime receives this value as one unit so a cloud Module cannot be paired
/// with a missing credential snapshot or a false microphone-upload disclosure.
/// A future local path can add its own constructor without adding path branches
/// to Runtime.
pub(crate) struct PreparedRecognition {
    module: RecognitionModule,
    credential: Option<RuntimeGenerationCredentialSnapshot>,
    uploads_microphone_audio: bool,
}

impl PreparedRecognition {
    pub(crate) fn cloud(
        module: RecognitionModule,
        credential: RuntimeGenerationCredentialSnapshot,
    ) -> Self {
        Self {
            module,
            credential: Some(credential),
            uploads_microphone_audio: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStartOutcome {
    Started,
    SupersededByStop,
}

struct RuntimeHandle {
    generation: RuntimeGeneration,
    publisher: Option<ChatboxPublication>,
    join_handle: JoinHandle<()>,
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
        install_generation: F,
    ) -> AppResult<RuntimeStartOutcome>
    where
        F: FnOnce(RuntimeGenerationSnapshot) -> AppResult<()>,
    {
        let RuntimeStartRequest {
            config,
            chatbox_pacer,
            caption_aggregate,
            chatbox_host_resolver,
            prepared_recognition,
            generation_id,
            config_revision,
            status_recorder,
            expected_stop_epoch,
        } = request;
        let PreparedRecognition {
            module: recognition_module,
            credential,
            uploads_microphone_audio,
        } = prepared_recognition;
        config.validate()?;
        let caption_pipeline_plan = plan_caption_pipeline(&config);
        let publication_timing = publication_timing_for_start(&caption_pipeline_plan)?;

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

        let generation = RuntimeGeneration::activate(&app, generation_id, caption_aggregate)?;
        let start_cancelled = || !self.stop_epoch_unchanged(expected_stop_epoch);
        let publisher_init = initialize_chatbox_publication(
            &app,
            &config.osc,
            publication_timing,
            chatbox_pacer,
            &generation,
            &chatbox_host_resolver,
            &start_cancelled,
        );
        if start_cancelled() {
            match &publisher_init {
                ChatboxPublicationInit::Ready(publisher) => {
                    let _ = generation.request_stop(Some(publisher));
                    let _ = publisher.join();
                }
                ChatboxPublicationInit::Disabled | ChatboxPublicationInit::Unavailable(_) => {
                    let _ = generation.request_stop(None);
                }
            }
            return Ok(RuntimeStartOutcome::SupersededByStop);
        }
        let requested_host = config.osc.host.clone();
        let requested_port = config.osc.port;
        let (publisher, chatbox_publication) = match publisher_init {
            ChatboxPublicationInit::Disabled => (
                None,
                ChatboxPublicationSnapshot::Disabled {
                    host: requested_host,
                    port: requested_port,
                },
            ),
            ChatboxPublicationInit::Ready(publisher) => (
                Some(publisher),
                ChatboxPublicationSnapshot::Ready {
                    host: requested_host,
                    port: requested_port,
                },
            ),
            ChatboxPublicationInit::Unavailable(error) => {
                emit_diagnostic(
                    &app,
                    DiagnosticUpdate::from_error(&error, "Chatbox OSC output could not start"),
                );
                (
                    None,
                    ChatboxPublicationSnapshot::Unavailable {
                        host: requested_host,
                        port: requested_port,
                        reason_code: error.code().to_string(),
                    },
                )
            }
        };

        let generation_snapshot = RuntimeGenerationSnapshot {
            id: generation_id,
            phase: RuntimeGenerationPhase::Starting,
            started_from_config_revision: config_revision,
            selection: RuntimeGenerationSelection::from(&config),
            caption_pipeline_plan,
            credential,
            chatbox_publication,
            uploads_microphone_audio,
        };
        if let Err(error) = install_generation(generation_snapshot) {
            let _ = generation.request_stop(publisher.as_ref());
            if let Some(publisher) = &publisher {
                let _ = publisher.join();
            }
            return Err(error);
        }

        let thread_generation = generation.clone();
        let thread_publisher = publisher.clone();
        let execution = RuntimeExecution::new(
            app,
            config.audio,
            recognition_module,
            thread_publisher,
            thread_generation,
            status_recorder,
        );
        let join_handle = thread::Builder::new()
            .name("vrc-live-caption-runtime".to_string())
            .spawn(move || run_runtime_thread(execution))
            .map_err(|error| AppError::runtime(format!("Failed to start runtime thread: {error}")));
        let join_handle = match join_handle {
            Ok(join_handle) => join_handle,
            Err(error) => {
                let _ = generation.request_stop(publisher.as_ref());
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

    pub(crate) fn stop<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        status_recorder: &RuntimeStatusRecorder,
    ) -> AppResult<()> {
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
                status_recorder,
                RuntimeStatus::Stopped,
                Some("Runtime is already stopped".to_string()),
            );
            return Ok(());
        };

        if let Err(error) = handle.generation.request_stop(handle.publisher.as_ref()) {
            handle.generation.cancel_work();
            emit_diagnostic(
                app,
                DiagnosticUpdate::from_error(&error, "Runtime outputs could not close"),
            );
        }
        record_and_emit_runtime_status(
            app,
            status_recorder,
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
                DiagnosticUpdate::from_error(&error, "Chatbox publication failed while stopping"),
            );
        }

        if runtime_panicked {
            let error = AppError::runtime("Runtime thread panicked while stopping.");
            record_and_emit_runtime_status(
                app,
                status_recorder,
                RuntimeStatus::Error,
                Some(error.to_string()),
            );
            return Err(error);
        }

        record_and_emit_runtime_status(
            app,
            status_recorder,
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

    if let Err(error) = handle
        .generation
        .close_outputs_for_runtime_error(handle.publisher.as_ref())
    {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Runtime outputs could not close"),
        );
    }
    if let Some(publisher) = &handle.publisher
        && let Err(error) = publisher.join()
    {
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Chatbox publication failed while closing"),
        );
    }

    handle
        .join_handle
        .join()
        .map_err(|_| AppError::runtime("Runtime thread panicked after stopping."))
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
