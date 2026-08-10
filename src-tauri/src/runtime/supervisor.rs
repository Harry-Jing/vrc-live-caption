//! Runtime-thread supervision and terminal output cleanup.

use super::RuntimeGeneration;
use super::coordinator::run_runtime;
use super::output::finish_runtime_output;
use crate::chatbox::{PublisherCloseReason, RuntimeChatboxPublisher};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, emit_diagnostic, record_and_emit_runtime_status,
};
use crate::host_resolver::HostResolver;
use crate::runtime_control::{RuntimeStatus, RuntimeStatusRecorder};
use secrecy::SecretString;
use tauri::{AppHandle, Runtime};

pub(super) fn run_runtime_thread<R: Runtime>(
    app: AppHandle<R>,
    config: AppConfig,
    openai_api_key: SecretString,
    publisher: Option<RuntimeChatboxPublisher>,
    generation: RuntimeGeneration,
    host_resolver: HostResolver,
    status_recorder: RuntimeStatusRecorder,
) {
    let error_generation = generation.clone();
    let cleanup_publisher = publisher.clone();
    let runtime_app = app.clone();
    let runtime_status_recorder = status_recorder.clone();

    supervise_runtime_thread(
        &app,
        &error_generation,
        cleanup_publisher.as_ref(),
        &status_recorder,
        move || {
            run_runtime(
                runtime_app,
                config,
                openai_api_key,
                publisher,
                generation,
                host_resolver,
                runtime_status_recorder,
            )
        },
    );
}

fn supervise_runtime_thread<R: Runtime>(
    app: &AppHandle<R>,
    generation: &RuntimeGeneration,
    publisher: Option<&RuntimeChatboxPublisher>,
    status_recorder: &RuntimeStatusRecorder,
    run: impl FnOnce() -> AppResult<()>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    let runtime_result = match outcome {
        Ok(runtime_result) => runtime_result,
        Err(panic) => {
            finish_runtime_output(app, generation, publisher, PublisherCloseReason::Stop);
            tracing::error!("runtime thread panicked; its generation and Publisher were stopped");
            record_and_emit_runtime_status(
                app,
                status_recorder,
                RuntimeStatus::Error,
                Some("Runtime thread panicked and was stopped".to_string()),
            );
            emit_diagnostic(
                app,
                DiagnosticUpdate::error(
                    DiagnosticCategory::Runtime,
                    "runtime.thread_panicked",
                    "Runtime thread panicked",
                    "The runtime generation was invalidated and pending Chatbox output was discarded.",
                ),
            );
            std::panic::resume_unwind(panic);
        }
    };

    let reason = if generation.is_hard_stop_requested() {
        PublisherCloseReason::Stop
    } else {
        PublisherCloseReason::RuntimeError
    };
    finish_runtime_output(app, generation, publisher, reason);

    if let Err(error) = runtime_result {
        tracing::warn!(
            code = error.code(),
            error_message = %error,
            "runtime stopped with error"
        );

        if generation.is_hard_stop_requested() {
            return;
        }

        record_and_emit_runtime_status(
            app,
            status_recorder,
            RuntimeStatus::Error,
            Some(error.to_string()),
        );
        emit_diagnostic(
            app,
            DiagnosticUpdate::from_error(&error, "Runtime stopped with an error"),
        );
    }
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
