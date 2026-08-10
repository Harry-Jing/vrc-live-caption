//! Tauri-managed application state.
//!
//! Holds the in-memory copy of the non-secret app config plus the runtime
//! manager. Saved-setting and credential operations are serialized with Start
//! here so their persisted and in-memory views cannot drift apart. Plaintext
//! secrets may be resolved transiently for Start, but are never stored in state
//! or in a frontend-facing snapshot.

use crate::audio::{AudioProbeRequest, AudioProbeResult, probe_audio_input as run_audio_probe};
use crate::capability_planner::{ResolvedPublicationPolicy, RuntimePlanSnapshot, plan_runtime};
use crate::caption_session::{CaptionSessionSnapshotV1, CaptionSessionStore};
use crate::chatbox::{ChatboxOscSender, ChatboxPacer, ChatboxSendReceipt, OSC_TEST_MESSAGE};
use crate::config::{AppConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, emit_diagnostic, emit_runtime_control_and_status,
};
use crate::host_resolver::HostResolver;
use crate::runtime::{RuntimeManager, RuntimeStartOutcome, RuntimeStartRequest};
use crate::runtime_control::{
    RuntimeControlSnapshot, RuntimeControlStore, RuntimeCredentialSnapshot, RuntimeStartSelection,
    RuntimeStatusRecorder,
};
use crate::saved_settings::{self, SavedSettingsLoad};
use crate::secrets::{
    delete_provider_secret, provider_secret_statuses, resolve_openai_api_key, save_provider_secret,
};
use std::sync::Mutex;
use tauri::{AppHandle, Runtime};

pub(super) struct AppState {
    control: RuntimeControlStore,
    // Serializes desired-state mutations with Start's configuration and
    // credential capture. Stop never waits for this gate: file or credential
    // store I/O must not delay the hard generation boundary.
    desired_state_gate: Mutex<()>,
    chatbox_pacer: ChatboxPacer,
    caption_session: CaptionSessionStore,
    host_resolver: HostResolver,
    runtime: RuntimeManager,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            control: RuntimeControlStore::default(),
            desired_state_gate: Mutex::new(()),
            chatbox_pacer: ChatboxPacer::default(),
            caption_session: CaptionSessionStore::default(),
            host_resolver: HostResolver::default(),
            runtime: RuntimeManager::default(),
        }
    }
}

impl AppState {
    pub(super) fn start_runtime(&self, app: &AppHandle) -> AppResult<RuntimeControlSnapshot> {
        let status_recorder = self.runtime_status_recorder();
        // Capture before waiting on desired-state I/O. Any later Stop changes
        // the epoch, so this invocation cannot install a runtime after Stop
        // returned while Start was blocked on config or credential work.
        let expected_stop_epoch = self.runtime.stop_epoch();
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;

        if !self.runtime.stop_epoch_unchanged(expected_stop_epoch) {
            return self.runtime_control_snapshot();
        }

        // Reject an active generation before touching the credential store.
        // A second Start must remain an idempotency error even if the key used
        // by the active session was deleted after that session began.
        self.runtime.prepare_for_start(app)?;

        let RuntimeStartSelection {
            config,
            config_requires_review,
            config_revision,
            credential_revision,
        } = self.control.start_selection()?;
        if let Err(error) = ensure_config_was_reviewed(config_requires_review) {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        if let Err(error) = config.validate() {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        let runtime_plan = plan_runtime(&config);
        if let Err(error) = ensure_runtime_plan_is_startable(&runtime_plan) {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        let resolved = match resolve_openai_api_key() {
            Ok(resolved) => resolved,
            Err(error) => {
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::error(
                        DiagnosticCategory::Config,
                        "config.openai_api_key_missing",
                        "Cloud STT is not configured",
                        error.to_string(),
                    ),
                );
                return self.finish_start_failure(app, error, None, expected_stop_epoch);
            }
        };
        let credential = RuntimeCredentialSnapshot {
            provider: SttProvider::OpenAi,
            storage: resolved.storage,
            display_suffix: resolved.display_suffix,
            revision: credential_revision,
        };

        let generation = self.control.allocate_generation()?;

        let start_result = self.runtime.start(
            app.clone(),
            RuntimeStartRequest {
                config,
                chatbox_pacer: self.chatbox_pacer.clone(),
                caption_session: self.caption_session.clone(),
                host_resolver: self.host_resolver.clone(),
                generation_id: generation,
                config_revision,
                openai_api_key: resolved.secret,
                credential,
                status_recorder,
                expected_stop_epoch,
            },
            |session| self.control.install_starting_session(session),
        );

        match start_result {
            Ok(RuntimeStartOutcome::Started | RuntimeStartOutcome::SupersededByStop) => {
                self.runtime_control_snapshot()
            }
            Err(error) => {
                self.finish_start_failure(app, error, Some(generation), expected_stop_epoch)
            }
        }
    }

    pub(super) fn stop_runtime<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> AppResult<RuntimeControlSnapshot> {
        // Deliberately do not hold the control-state lock while RuntimeManager
        // joins: the worker may emit its final status through that short lock.
        let status_recorder = self.runtime_status_recorder();
        self.runtime.stop(app, &status_recorder)?;
        self.runtime_control_snapshot()
    }

    pub(super) fn runtime_control_snapshot(&self) -> AppResult<RuntimeControlSnapshot> {
        self.control.snapshot()
    }

    pub(super) fn caption_session_snapshot(&self) -> AppResult<CaptionSessionSnapshotV1> {
        self.caption_session.snapshot()
    }

    pub(super) fn probe_audio_input<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        request: &AudioProbeRequest,
    ) -> AppResult<AudioProbeResult> {
        let _probe_lease = self.runtime.begin_audio_probe(app)?;
        match run_audio_probe(request) {
            Ok(result) => {
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::info(
                        DiagnosticCategory::Audio,
                        "audio.probe_completed",
                        "Microphone test completed",
                        format!(
                            "Observed local microphone levels for {} ms at {} Hz; no audio left the app.",
                            result.duration_ms, result.sample_rate
                        ),
                    ),
                );
                Ok(result)
            }
            Err(error) => {
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::from_error(&error, "Microphone test failed"),
                );
                Err(error)
            }
        }
    }

    pub(super) fn send_osc_test_message(&self) -> AppResult<ChatboxSendReceipt> {
        let osc_config = self.control.effective_osc_config()?;
        let sender = ChatboxOscSender::new(&osc_config, &self.host_resolver, &|| false)?;
        self.chatbox_pacer
            .wait_for_turn(None)?
            .ok_or_else(|| AppError::state("OSC Test pacing was cancelled."))?
            .attempt(|| sender.send_text(OSC_TEST_MESSAGE))
    }

    fn runtime_status_recorder(&self) -> RuntimeStatusRecorder {
        self.control.status_recorder()
    }

    #[cfg(test)]
    fn record_start_error(
        &self,
        error: &AppError,
        installed_generation: Option<u64>,
    ) -> AppResult<RuntimeControlSnapshot> {
        self.control.record_start_error(error, installed_generation)
    }

    fn record_start_error_if_current(
        &self,
        error: &AppError,
        installed_generation: Option<u64>,
        expected_stop_epoch: u64,
    ) -> AppResult<Option<RuntimeControlSnapshot>> {
        self.control
            .record_start_error_if_current(error, installed_generation, || {
                self.runtime.stop_epoch_unchanged(expected_stop_epoch)
            })
    }

    fn finish_start_failure<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        error: AppError,
        installed_generation: Option<u64>,
        expected_stop_epoch: u64,
    ) -> AppResult<RuntimeControlSnapshot> {
        match self.record_start_error_if_current(
            &error,
            installed_generation,
            expected_stop_epoch,
        )? {
            Some(snapshot) => {
                emit_runtime_control_and_status(app, snapshot);
                Err(error)
            }
            None => self.runtime_control_snapshot(),
        }
    }

    pub(super) fn load_config<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<AppConfig> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        let (config, config_requires_review) = match saved_settings::load(app)? {
            // A corrupt or invalid config file must not lock the user out of
            // the Settings page (the form only renders with a loaded config),
            // so fall back to defaults and report it; the next save replaces
            // the broken file.
            SavedSettingsLoad::Ready(config) => (config, false),
            SavedSettingsLoad::DefaultsRequireReview {
                config,
                path,
                error,
            } => {
                tracing::warn!(
                    path = %path.display(),
                    error_message = %error,
                    "config file is unusable; defaults loaded"
                );

                emit_diagnostic(
                    app,
                    DiagnosticUpdate::error(
                        DiagnosticCategory::Config,
                        "config.defaults_loaded",
                        "Saved settings could not be loaded",
                        format!(
                            "The settings file at {} is unusable: {error} Default settings are in \
                             use; saving settings replaces the file.",
                            path.display()
                        ),
                    ),
                );

                (config, true)
            }
        };

        self.control.replace_loaded_config(
            config.clone(),
            config_requires_review,
            provider_secret_statuses(),
        )?;

        Ok(config)
    }

    pub(super) fn save_config(
        &self,
        app: &AppHandle,
        config: AppConfig,
    ) -> AppResult<RuntimeControlSnapshot> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        saved_settings::save(app, &config)?;

        self.control.replace_saved_config(config)
    }

    pub(super) fn save_provider_secret(
        &self,
        provider: SttProvider,
        secret: String,
    ) -> AppResult<RuntimeControlSnapshot> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        save_provider_secret(provider, secret)?;
        self.control
            .replace_provider_secret_statuses(provider_secret_statuses())
    }

    pub(super) fn delete_provider_secret(
        &self,
        provider: SttProvider,
    ) -> AppResult<RuntimeControlSnapshot> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        delete_provider_secret(provider)?;
        self.control
            .replace_provider_secret_statuses(provider_secret_statuses())
    }
}

fn ensure_runtime_plan_is_startable(plan: &RuntimePlanSnapshot) -> AppResult<()> {
    match plan.publication.resolved_policy() {
        Some(ResolvedPublicationPolicy::Completed | ResolvedPublicationPolicy::LiveUnit { .. }) => {
            Ok(())
        }
        None => Err(AppError::config(format!(
            "The selected recognition path and publication mode are incompatible ({}).",
            plan.publication
                .incompatibility_code()
                .unwrap_or("publication.incompatible")
        ))),
    }
}

fn ensure_config_was_reviewed(config_requires_review: bool) -> AppResult<()> {
    if config_requires_review {
        Err(AppError::config(
            "Saved settings use a removed or invalid configuration. Review and save the current settings before starting recognition.",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
